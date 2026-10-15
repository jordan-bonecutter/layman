//! The helper owns mirror windows, including after confirmation. Closing its
//! input ends the presentation; an unconfirmed change has an independent timer.
use crate::{duplication::Duplication, preview::SECONDS};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Settings {
    source: String,
    targets: Vec<String>,
}

#[derive(Serialize, Deserialize)]
enum Action {
    Preview(Settings),
    Keep,
    Revert,
    Stop,
}

#[derive(Serialize, Deserialize)]
struct Request {
    id: u64,
    action: Action,
}

#[derive(Serialize, Deserialize)]
struct Event {
    id: Option<u64>,
    settings: Settings,
    pending: bool,
    error: Option<String>,
    message: String,
}

struct Helper {
    child: Child,
    input: Option<ChildStdin>,
    events: mpsc::Receiver<Result<Event>>,
}

impl Helper {
    fn spawn() -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--duplication-guard")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let output = child
            .stdout
            .take()
            .context("Missing duplication helper output")?;
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let result = line
                    .map_err(anyhow::Error::from)
                    .and_then(|line| serde_json::from_str(&line).map_err(Into::into));
                if send.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
        })
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct Preview {
    helper: Option<Helper>,
    pub source: String,
    targets: Vec<String>,
    started: Option<Instant>,
    next_id: u64,
}

impl Preview {
    pub fn targets(&self) -> Vec<String> {
        self.targets.clone()
    }

    pub fn pending(&self) -> bool {
        self.started.is_some()
    }

    pub fn remaining(&self) -> u64 {
        self.started.map_or(0, |started| {
            SECONDS.saturating_sub(started.elapsed().as_secs())
        })
    }

    fn apply_event(&mut self, event: &Event) {
        self.source.clone_from(&event.settings.source);
        self.targets.clone_from(&event.settings.targets);
        if event.pending {
            self.started.get_or_insert_with(Instant::now);
        } else {
            self.started = None;
        }
    }

    fn request(&mut self, action: Action) -> Result<()> {
        if self.helper.is_none() {
            self.helper = Some(Helper::spawn()?);
        }
        self.next_id += 1;
        let id = self.next_id;
        let input = self
            .helper
            .as_mut()
            .unwrap()
            .input
            .as_mut()
            .context("Duplication helper closed")?;
        serde_json::to_writer(&mut *input, &Request { id, action })?;
        writeln!(input)?;
        input.flush()?;
        loop {
            let event = self
                .helper
                .as_ref()
                .unwrap()
                .events
                .recv_timeout(Duration::from_secs(3))
                .context("Duplication helper did not respond")??;
            self.apply_event(&event);
            if event.id == Some(id) {
                if let Some(error) = event.error {
                    bail!("{error}");
                }
                return Ok(());
            }
        }
    }

    pub fn start(&mut self, source: &str, targets: &[String]) -> Result<()> {
        self.request(Action::Preview(Settings {
            source: source.into(),
            targets: targets.into(),
        }))
    }

    pub fn finish(&mut self, keep: bool) -> Result<()> {
        self.request(if keep { Action::Keep } else { Action::Revert })
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.helper.is_some() {
            self.request(Action::Stop)?;
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Option<String> {
        let mut message = None;
        loop {
            let result = self.helper.as_ref()?.events.try_recv();
            match result {
                Ok(Ok(event)) => {
                    self.apply_event(&event);
                    message = Some(event.error.unwrap_or(event.message));
                }
                Ok(Err(error)) => message = Some(format!("Duplication helper: {error:#}")),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.helper.take();
                    self.source.clear();
                    self.targets.clear();
                    self.started = None;
                    message = Some("Duplication helper exited; presentation ended.".into());
                    break;
                }
            }
        }
        message
    }
}

#[derive(Default)]
struct Guard {
    session: Duplication,
    before: Option<(Settings, Instant)>,
}

impl Guard {
    fn settings(&self) -> Settings {
        Settings {
            source: self.session.source.clone(),
            targets: self.session.targets(),
        }
    }

    fn revert(&mut self) -> Result<()> {
        if let Some((settings, _)) = self.before.take()
            && let Err(error) = self.session.start(&settings.source, &settings.targets)
        {
            self.session.stop();
            bail!("Could not restore previous duplication; all mirrors stopped: {error:#}");
        }
        Ok(())
    }

    fn action(&mut self, action: Action) -> Result<&'static str> {
        match action {
            Action::Preview(settings) => {
                if self.before.is_some() {
                    bail!("Confirm or revert the current duplication preview first.");
                }
                let before = self.settings();
                self.session.start(&settings.source, &settings.targets)?;
                self.before = Some((before, Instant::now()));
                Ok("Confirm duplication within 20 seconds.")
            }
            Action::Keep => {
                if let Some(message) = self.session.poll() {
                    self.revert()?;
                    bail!("{message}");
                }
                if self.before.take().is_none() {
                    bail!("Duplication preview expired. Previous session restored.");
                }
                Ok("Duplication confirmed.")
            }
            Action::Revert => {
                self.revert()?;
                Ok("Previous duplication restored.")
            }
            Action::Stop => {
                self.before = None;
                self.session.stop();
                Ok("Duplication stopped.")
            }
        }
    }

    fn emit(&self, id: Option<u64>, result: Result<&str>) -> Result<()> {
        let (message, error) = match result {
            Ok(message) => (message.into(), None),
            Err(error) => (String::new(), Some(format!("{error:#}"))),
        };
        let mut output = std::io::stdout().lock();
        serde_json::to_writer(
            &mut output,
            &Event {
                id,
                settings: self.settings(),
                pending: self.before.is_some(),
                error,
                message,
            },
        )?;
        writeln!(output)?;
        output.flush()?;
        Ok(())
    }
}

pub fn guard() -> Result<()> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let Ok(request) = serde_json::from_str::<Request>(&line) else {
                break;
            };
            if send.send(request).is_err() {
                break;
            }
        }
    });
    let mut guard = Guard::default();
    loop {
        // Check the deadline before commands so a late Keep cannot win the race.
        if guard
            .before
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() >= Duration::from_secs(SECONDS))
        {
            let result = guard
                .revert()
                .map(|_| "Duplication preview expired. Previous session restored.");
            guard.emit(None, result)?;
        }
        if let Some(message) = guard.session.poll() {
            let result = guard.revert().and_then(|_| Err(anyhow::anyhow!(message)));
            guard.emit(None, result)?;
        }
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(request) => {
                // The receive may cross the deadline.
                if guard
                    .before
                    .as_ref()
                    .is_some_and(|(_, started)| started.elapsed() >= Duration::from_secs(SECONDS))
                {
                    let result = guard
                        .revert()
                        .map(|_| "Duplication preview expired. Previous session restored.");
                    guard.emit(None, result)?;
                }
                let result = guard.action(request.action);
                guard.emit(Some(request.id), result)?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}
