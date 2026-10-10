use crate::{ipc, model::Layout, store};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

pub const SECONDS: u64 = 20;
#[derive(Serialize, Deserialize)]
struct Transaction {
    before: Layout,
    candidate: Layout,
}
pub struct Preview {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    _file: NamedTempFile,
    pub started: Instant,
}
impl Preview {
    pub fn start(candidate: &Layout) -> Result<Self> {
        candidate.validate()?;
        let before = ipc::snapshot()?;
        // Validate all targets and modes before starting the rollback helper.
        candidate.connected(&before)?;
        let mut file = NamedTempFile::new()?;
        serde_json::to_writer(
            &mut file,
            &Transaction {
                before,
                candidate: candidate.clone(),
            },
        )?;
        file.flush()?;
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--guard")
            .arg(file.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().context("Missing guard output")?);
        let mut preview = Self {
            child,
            input,
            output,
            _file: file,
            started: Instant::now(),
        };
        let mut ready = String::new();
        preview.output.read_line(&mut ready)?;
        if ready.trim() != "ready" {
            bail!("Rollback helper could not start: {}", ready.trim());
        }
        preview.started = Instant::now();
        Ok(preview)
    }
    pub fn remaining(&self) -> u64 {
        SECONDS.saturating_sub(self.started.elapsed().as_secs())
    }
    pub fn finish(&mut self, keep: bool) -> Result<()> {
        if let Some(mut input) = self.input.take() {
            // The helper arbitrates confirmation vs timeout, and writes the saved layout itself.
            let _ = writeln!(input, "{}", if keep { "keep" } else { "revert" });
        }
        let mut reply = String::new();
        self.output.read_line(&mut reply)?;
        self.child.wait()?;
        match reply.trim() {
            "kept" if keep => Ok(()),
            "reverted" if !keep => Ok(()),
            "reverted" => bail!("Preview expired. Previous layout restored."),
            other => bail!(
                "Rollback helper: {other}. Check layman/rollback.log in your config directory."
            ),
        }
    }
}
// Closing the GUI (or killing it) closes the helper's stdin, causing independent rollback.
impl Drop for Preview {
    fn drop(&mut self) {
        self.input.take();
    }
}
pub fn guard(path: &std::path::Path) -> Result<()> {
    let _lock = store::mutation_lock()?.context("Another Layman preview or restore is active")?;
    let mut transaction: Transaction = serde_json::from_slice(&std::fs::read(path)?)?;
    // Capture after acquiring the lock: a previous preview may have finished
    // between the editor's preflight snapshot and spawning this helper.
    transaction.before = ipc::snapshot()?;
    transaction.candidate.validate()?;
    if let Err(e) = ipc::apply(&transaction.candidate) {
        let recovery = rollback(&transaction.before);
        bail!("Cannot apply preview: {e:#}; rollback: {recovery:?}");
    }
    println!("ready");
    std::io::stdout().flush()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let _ = tx.send(line);
    });
    let answer = rx
        .recv_timeout(Duration::from_secs(SECONDS))
        .unwrap_or_default();
    if answer.trim() == "keep" {
        match ipc::apply(&transaction.candidate).and_then(|_| store::save(&transaction.candidate)) {
            Ok(()) => {
                println!("kept");
                return Ok(());
            }
            Err(e) => {
                rollback(&transaction.before)?;
                println!("Could not save: {e:#}. Previous layout restored.");
                return Ok(());
            }
        }
    }
    match rollback(&transaction.before) {
        Ok(()) => println!("reverted"),
        Err(e) => {
            println!("Rollback failed: {e:#}");
            return Err(e);
        }
    }
    Ok(())
}
fn rollback(layout: &Layout) -> Result<()> {
    let result = ipc::apply(layout);
    if let Err(e) = &result
        && let Ok(dir) = store::directory()
    {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("rollback.log"), format!("{e:#}\n"));
    }
    result
}
