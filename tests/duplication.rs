use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Guard {
    child: Child,
    input: Option<ChildStdin>,
    events: mpsc::Receiver<Value>,
    dir: tempfile::TempDir,
    id: u64,
}

impl Guard {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("wl-mirror");
        std::fs::write(&binary, "#!/bin/sh\nprintf '%s %s %s\\n' \"$$\" \"$3\" \"$2\" >> \"$MIRROR_LOG\"\nif [ \"$2\" = FAIL ]; then echo 'capture denied' >&2; exit 1; fi\nexec sleep 90\n").unwrap();
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_layman"))
            .arg("--duplication-guard")
            .env(
                "PATH",
                std::env::join_paths([
                    dir.path(),
                    std::path::Path::new("/usr/bin"),
                    std::path::Path::new("/bin"),
                ])
                .unwrap(),
            )
            .env("MIRROR_LOG", dir.path().join("log"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let event = serde_json::from_str(&line.unwrap()).unwrap();
                if send.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            events,
            dir,
            id: 0,
        }
    }

    fn request(&mut self, action: Value) -> Value {
        self.id += 1;
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{}", json!({"id": self.id, "action": action})).unwrap();
        input.flush().unwrap();
        loop {
            let event = self.event();
            if event["id"] == self.id {
                return event;
            }
        }
    }

    fn event(&self) -> Value {
        self.events.recv_timeout(Duration::from_secs(25)).unwrap()
    }

    fn preview(&mut self, source: &str, targets: &[&str]) -> Value {
        let event = self.request(json!({"Preview": {"source": source, "targets": targets}}));
        assert!(event["error"].is_null(), "{event}");
        assert_eq!(event["pending"], true);
        event
    }

    fn pids(&self, count: usize) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let log = std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default();
            let pids: Vec<_> = log
                .lines()
                .filter_map(|line| line.split_whitespace().next()?.parse().ok())
                .collect();
            if pids.len() >= count {
                return pids;
            }
            assert!(Instant::now() < deadline, "Mirror did not start: {log}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn close(&mut self) {
        self.input.take();
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.wait();
    }
}

fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[test]
fn timeout_restores_confirmed_session_without_gui_activity_and_rejects_late_confirmation() {
    let mut guard = Guard::new();
    guard.preview("eDP-1", &["DP-1"]);
    assert_eq!(guard.request(json!("Keep"))["pending"], false);
    guard.pids(1);
    guard.preview("DP-1", &["HDMI-A-1"]);
    let candidate = guard.pids(2)[1];
    // No commands or UI polling: the separate helper must enforce the real deadline.
    let reverted = guard.event();
    assert!(reverted["message"].as_str().unwrap().contains("expired"));
    assert_eq!(
        reverted["settings"],
        json!({"source": "eDP-1", "targets": ["DP-1"]})
    );
    assert_eq!(reverted["pending"], false);
    assert!(!alive(candidate));
    assert!(
        guard.request(json!("Keep"))["error"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
    let pids = guard.pids(3);
    assert!(alive(pids[2]));
    guard.close();
    assert!(pids.into_iter().all(|pid| !alive(pid)));
}

#[test]
fn reverts_new_sessions_destination_edits_and_capture_failures_and_cleans_up_on_disconnect() {
    let mut guard = Guard::new();
    guard.preview("eDP-1", &["DP-1"]);
    let first = guard.pids(1)[0];
    let reverted = guard.request(json!("Revert"));
    assert_eq!(reverted["settings"], json!({"source": "", "targets": []}));
    assert!(!alive(first));
    guard.preview("eDP-1", &["DP-1", "HDMI-A-1"]);
    guard.pids(3);
    assert!(guard.request(json!("Keep"))["error"].is_null());
    guard.preview("eDP-1", &["DP-1"]);
    let reverted = guard.request(json!("Revert"));
    assert_eq!(reverted["settings"]["targets"], json!(["DP-1", "HDMI-A-1"]));
    guard.pids(4);
    guard.preview("DP-1", &["FAIL"]);
    let failed = guard.event();
    assert!(failed["error"].as_str().unwrap().contains("capture denied"));
    assert_eq!(failed["pending"], false);
    assert_eq!(
        failed["settings"],
        json!({"source": "eDP-1", "targets": ["DP-1", "HDMI-A-1"]})
    );
    let pids = guard.pids(7);
    // EOF is what the helper observes when the GUI closes or is killed.
    guard.close();
    assert!(pids.into_iter().all(|pid| !alive(pid)));
}

#[test]
fn unconfirmed_windows_are_removed_when_gui_connection_closes() {
    let mut guard = Guard::new();
    guard.preview("eDP-1", &["DP-1", "HDMI-A-1"]);
    let pids = guard.pids(2);
    guard.close();
    assert!(pids.into_iter().all(|pid| !alive(pid)));
}
