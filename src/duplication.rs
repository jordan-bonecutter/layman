use anyhow::{Context, Result, bail};
use std::{
    io::Read,
    process::{Child, Command, Stdio},
};

pub fn available() -> bool {
    available_on_path(std::env::var_os("PATH").as_deref())
}

fn available_on_path(path: Option<&std::ffi::OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| {
            std::fs::metadata(dir.join("wl-mirror"))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

struct Mirror {
    target: String,
    child: Child,
    errors: tempfile::NamedTempFile,
}

impl Drop for Mirror {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Presentation sessions belong to the editor and end when it closes.
#[derive(Default)]
pub struct Duplication {
    pub source: String,
    mirrors: Vec<Mirror>,
}

impl Duplication {
    pub fn targets(&self) -> Vec<String> {
        self.mirrors.iter().map(|m| m.target.clone()).collect()
    }

    pub fn stop(&mut self) {
        self.mirrors.clear();
        self.source.clear();
    }

    pub fn start(&mut self, source: &str, targets: &[String]) -> Result<()> {
        self.start_with(std::path::Path::new("wl-mirror"), source, targets)
    }

    fn start_with(
        &mut self,
        program: &std::path::Path,
        source: &str,
        targets: &[String],
    ) -> Result<()> {
        if source.starts_with('-') || targets.iter().any(|target| target.starts_with('-')) {
            bail!("Unsupported display name.");
        }
        if targets.iter().any(|target| target == source) {
            bail!("A display cannot duplicate itself.");
        }
        // Keep existing windows when editing a session; avoid stacking mirrors.
        let mut added = Vec::new();
        for target in targets {
            if self.source == source && self.mirrors.iter().any(|m| &m.target == target) {
                continue;
            }
            let errors = tempfile::NamedTempFile::new()?;
            let child = Command::new(program)
                .args(["--fullscreen-output", target, source])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(errors.reopen()?)
                .spawn()
                .context("Could not start screen duplication. Install wl-mirror and try again")?;
            added.push(Mirror {
                target: target.clone(),
                child,
                errors,
            });
        }
        self.mirrors
            .retain(|m| self.source == source && targets.contains(&m.target));
        self.mirrors.extend(added);
        self.source = if targets.is_empty() {
            String::new()
        } else {
            source.into()
        };
        Ok(())
    }

    pub fn poll(&mut self) -> Option<String> {
        let mut message = None;
        self.mirrors
            .retain_mut(|mirror| match mirror.child.try_wait() {
                Ok(None) => true,
                result => {
                    let mut details = String::new();
                    if let Ok(file) = mirror.errors.reopen() {
                        let _ = file.take(4096).read_to_string(&mut details);
                    }
                    message = Some(match result {
                        Ok(Some(status)) if status.success() => {
                            format!("Duplication to {} ended.", mirror.target)
                        }
                        _ => format!(
                            "Duplication to {} stopped. {}",
                            mirror.target,
                            details.trim()
                        ),
                    });
                    false
                }
            });
        if self.mirrors.is_empty() {
            self.source.clear();
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };

    #[test]
    fn dependency_detection_requires_an_executable_file_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = Some(dir.path().as_os_str());
        assert!(!available_on_path(None));
        assert!(!available_on_path(path));
        let binary = dir.path().join("wl-mirror");
        std::fs::write(&binary, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!available_on_path(path));
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(available_on_path(path));
        let directories = tempfile::tempdir().unwrap();
        std::fs::create_dir(directories.path().join("wl-mirror")).unwrap();
        assert!(!available_on_path(Some(directories.path().as_os_str())));
    }

    fn fixture(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mirror");
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }

    #[test]
    fn launches_each_destination_reuses_windows_and_reaps_on_stop() {
        let (dir, program) =
            fixture("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/$2\"\nexec sleep 60\n");
        let mut session = Duplication::default();
        session
            .start_with(&program, "eDP-1", &["DP-1".into(), "HDMI-A-1".into()])
            .unwrap();
        let pids: Vec<_> = session.mirrors.iter().map(|m| m.child.id()).collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dir.path().join("HDMI-A-1").exists() || !dir.path().join("DP-1").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("DP-1")).unwrap(),
            "--fullscreen-output\nDP-1\neDP-1\n"
        );
        session
            .start_with(&program, "eDP-1", &["DP-1".into()])
            .unwrap();
        assert_eq!(session.mirrors[0].child.id(), pids[0]);
        assert!(!std::path::Path::new(&format!("/proc/{}", pids[1])).exists());
        session.stop();
        assert!(session.targets().is_empty());
        assert!(!std::path::Path::new(&format!("/proc/{}", pids[0])).exists());
    }

    #[test]
    fn reports_capture_failures_and_clears_session() {
        let (_dir, program) = fixture("#!/bin/sh\necho 'capture unavailable' >&2\nexit 1\n");
        let mut session = Duplication::default();
        session
            .start_with(&program, "eDP-1", &["DP-1".into()])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let message = loop {
            if let Some(message) = session.poll() {
                break message;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(message.contains("capture unavailable"));
        assert!(session.source.is_empty());
    }

    #[test]
    fn missing_dependency_and_self_duplication_are_errors() {
        let mut session = Duplication::default();
        assert!(
            session
                .start_with(
                    std::path::Path::new("/nonexistent/wl-mirror"),
                    "eDP-1",
                    &["DP-1".into()]
                )
                .unwrap_err()
                .to_string()
                .contains("Install wl-mirror")
        );
        assert!(session.start("eDP-1", &["eDP-1".into()]).is_err());
        assert!(session.targets().is_empty());
    }
}
