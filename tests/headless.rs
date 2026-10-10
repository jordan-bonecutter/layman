//! Real compositor tests, isolated from the desktop. Run with:
//! cargo test --test headless -- --ignored --nocapture
mod support;
use anyhow::{Context, Result, ensure};
use layman::{ipc, model::Layout, store};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Sway {
    child: Child,
    dir: tempfile::TempDir,
    runtime: PathBuf,
    socket: PathBuf,
    config: PathBuf,
}
impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Sway {
    fn start() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let runtime = dir.path().join("runtime");
        fs::create_dir(&runtime)?;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
        let config = dir.path().join("sway-test.conf");
        fs::write(
            &config,
            "xwayland disable\noutput HEADLESS-1 mode 1280x800 pos 0 0\noutput HEADLESS-2 mode 1280x800 pos 1280 0\nworkspace 1 output HEADLESS-1\nfor_window [app_id=\"dev.layman.Layman\"] fullscreen enable\n",
        )?;
        let log = fs::File::create(dir.path().join("sway.log"))?;
        let mut command = Command::new("sway");
        command
            .args(["--unsupported-gpu", "-c"])
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "2")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SWAYSOCK")
            .stdout(Stdio::null())
            .stderr(log);
        let mut child = command.spawn()?;
        let start = Instant::now();
        let socket = loop {
            let socket = fs::read_dir(&runtime)?
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy().starts_with("sway-ipc."))
                .map(|e| e.path());
            if let Some(socket) = socket
                && ipc::request_at::<serde_json::Value>(&socket, 3, "").is_ok()
            {
                break socket;
            }
            if let Some(status) = child.try_wait()? {
                anyhow::bail!(
                    "Headless Sway exited {status}: {}",
                    fs::read_to_string(dir.path().join("sway.log"))?
                );
            }
            ensure!(
                start.elapsed() < Duration::from_secs(10),
                "Headless Sway startup timed out"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        Ok(Self {
            child,
            dir,
            runtime,
            socket,
            config,
        })
    }
    fn command(&self, command: &str) -> Result<()> {
        let reply: serde_json::Value = ipc::request_at(&self.socket, 0, command)?;
        ensure!(
            reply
                .as_array()
                .context("command array")?
                .iter()
                .all(|r| r["success"] == true),
            "Sway rejected {command}: {reply}"
        );
        Ok(())
    }
    fn app(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_layman"));
        c.env("SWAYSOCK", &self.socket)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        if let Some(wayland) = fs::read_dir(&self.runtime)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name().to_string_lossy().starts_with("wayland-")
                    && !e.file_name().to_string_lossy().ends_with(".lock")
            })
        {
            c.env("WAYLAND_DISPLAY", wayland.file_name());
        }
        c
    }
    fn snapshot(&self) -> Result<Layout> {
        let output = self.app().arg("--inspect").output()?;
        ensure!(
            output.status.success(),
            "inspect: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    fn guard(
        &self,
        before: &Layout,
        candidate: &Layout,
    ) -> Result<(Child, BufReader<std::process::ChildStdout>)> {
        let path = self.dir.path().join("transaction.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({ "before": before, "candidate": candidate }))?,
        )?;
        let mut guard = self
            .app()
            .arg("--guard")
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut output = BufReader::new(guard.stdout.take().unwrap());
        let mut ready = String::new();
        output.read_line(&mut ready)?;
        ensure!(ready.trim() == "ready", "Guard failed to start: {ready}");
        Ok((guard, output))
    }
    fn apply_outputs(&self, l: &Layout) -> Result<()> {
        for c in l.output_commands(false)? {
            self.command(&c)?;
        }
        Ok(())
    }
}
fn equivalent_outputs(a: &Layout, b: &Layout) {
    for o in &a.outputs {
        let other = b.outputs.iter().find(|p| p.name == o.name).unwrap();
        assert_eq!(
            (o.active, o.rect.x, o.rect.y, o.scale, &o.transform),
            (
                other.active,
                other.rect.x,
                other.rect.y,
                other.scale,
                &other.transform
            )
        );
    }
}
#[test]
#[ignore = "requires sway, software EGL, and grim; runs its own headless compositor"]
fn real_headless_sway_ui_persistence_and_rollback() -> Result<()> {
    let sway = Sway::start()?;
    let before = sway.snapshot()?;
    ensure!(before.outputs.len() == 2, "Expected two headless displays");
    println!("✓ isolated headless compositor and read-only IPC");
    let mut candidate = before.clone();
    candidate.outputs[1].rect.x = 0;
    candidate.outputs[1].rect.y = 800;
    let (mut guard, mut reply) = sway.guard(&before, &candidate)?;
    sway.apply_outputs(&candidate)?;
    equivalent_outputs(&candidate, &sway.snapshot()?);
    // EOF is the same recovery signal as closing/crashing the GUI.
    guard.stdin.take();
    let mut line = String::new();
    reply.read_line(&mut line)?;
    ensure!(line.trim() == "reverted", "EOF rollback: {line}");
    guard.wait()?;
    equivalent_outputs(&before, &sway.snapshot()?);
    println!("✓ independent helper restores displays after GUI connection loss");

    let (mut guard, mut reply) = sway.guard(&before, &candidate)?;
    sway.apply_outputs(&candidate)?;
    writeln!(guard.stdin.as_mut().unwrap(), "keep")?;
    let mut line = String::new();
    reply.read_line(&mut line)?;
    ensure!(line.trim() == "kept", "Keep: {line}");
    guard.wait()?;
    equivalent_outputs(&candidate, &sway.snapshot()?);
    let saved_dir = sway.dir.path().join("config/layman");
    ensure!(store::load_at(&saved_dir)?.is_some(), "No saved layout");
    println!("✓ confirmation applies and atomically persists layout");

    // Include AFTER conflicting output commands. Verify real Sway config parsing.
    let mut file = fs::OpenOptions::new().append(true).open(&sway.config)?;
    writeln!(file, "include {}", saved_dir.join("sway.conf").display())?;
    sway.command("output HEADLESS-2 pos 2000 0")?;
    sway.command("reload")?;
    std::thread::sleep(Duration::from_millis(400));
    equivalent_outputs(&candidate, &sway.snapshot()?);
    println!("✓ Sway reload preserves saved layout through generated include");

    // Launch the native renderer entirely inside the headless Wayland server.
    let ui_log = fs::File::create(sway.dir.path().join("ui.log"))?;
    let mut gui = sway.app().stdout(Stdio::null()).stderr(ui_log).spawn()?;
    let gui_result = (|| -> Result<()> {
        let start = Instant::now();
        loop {
            ensure!(
                gui.try_wait()?.is_none(),
                "Native GUI exited: {}",
                fs::read_to_string(sway.dir.path().join("ui.log"))?
            );
            let tree: serde_json::Value = ipc::request_at(&sway.socket, 4, "")?;
            if tree.to_string().contains("dev.layman.Layman") {
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(10),
                "GUI startup timeout"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(500));
        let wayland_path = fs::read_dir(&sway.runtime)?
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name().to_string_lossy().starts_with("wayland-")
                    && !e.file_name().to_string_lossy().ends_with(".lock")
            })
            .context("Wayland socket")?
            .path();
        let pointer = support::Pointer::new(&wayland_path)?;
        // Drag the lower display in the actual canvas, preview it, then revert.
        pointer.motion(500, 550)?;
        pointer.button(true)?;
        std::thread::sleep(Duration::from_millis(80));
        pointer.motion(570, 550)?;
        pointer.button(false)?;
        std::thread::sleep(Duration::from_millis(200));
        pointer.click(1190, 740)?;
        let dragged = sway.snapshot()?;
        ensure!(
            dragged.outputs[1].rect.x > 0,
            "Canvas drag did not change the display position"
        );
        pointer.click(225, 735)?; // Revert now.
        equivalent_outputs(&candidate, &sway.snapshot()?);
        println!("✓ actual canvas drag previews correctly; Revert restores the layout");
        pointer.click(1070, 740)?; // Reset draft edits.
        pointer.click(90, 210)?; // Arrange in a row, through the actual UI.
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts");
        fs::create_dir_all(&artifacts)?;
        let wayland = fs::read_dir(&sway.runtime)?
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name().to_string_lossy().starts_with("wayland-")
                    && !e.file_name().to_string_lossy().ends_with(".lock")
            })
            .unwrap()
            .file_name();
        let status = Command::new("grim")
            .args(["-o", "HEADLESS-1"])
            .arg(artifacts.join("headless-ui.png"))
            .env("XDG_RUNTIME_DIR", &sway.runtime)
            .env("WAYLAND_DISPLAY", &wayland)
            .status()?;
        ensure!(status.success(), "Screenshot failed");
        println!("✓ native Wayland/OpenGL UI rendered; artifacts/headless-ui.png");
        pointer.click(1190, 740)?; // Preview layout.
        let row = sway.snapshot()?;
        ensure!(
            row.outputs[1].rect.x == 1280 && row.outputs[1].rect.y == 0,
            "UI Preview did not apply Arrange in a row"
        );
        let status = Command::new("grim")
            .args(["-o", "HEADLESS-1"])
            .arg(artifacts.join("headless-preview.png"))
            .env("XDG_RUNTIME_DIR", &sway.runtime)
            .env("WAYLAND_DISPLAY", &wayland)
            .status()?;
        ensure!(status.success(), "Preview screenshot failed");
        pointer.click(350, 735)?; // Keep & save.
        let saved = store::load_at(&saved_dir)?.context("UI did not save")?;
        ensure!(
            saved.outputs[1].rect.x == 1280 && saved.outputs[1].rect.y == 0,
            "UI confirmation did not persist the arranged layout"
        );
        println!("✓ actual GUI Arrange → Preview → Keep interaction applies and saves");

        // Move a workspace containing a real Wayland window. Verify the window
        // remains alive and the workspace is assigned to the requested output.
        let live = sway.snapshot()?;
        let mut moved = live.clone();
        let gui_workspace = moved
            .workspaces
            .iter_mut()
            .find(|w| w.focused)
            .context("GUI workspace")?;
        let target = if gui_workspace.output == "HEADLESS-1" {
            "HEADLESS-2"
        } else {
            "HEADLESS-1"
        };
        let workspace_name = gui_workspace.name.clone();
        gui_workspace.output = target.into();
        store::save_at(&saved_dir, &moved, Path::new(env!("CARGO_BIN_EXE_layman")))?;
        let result = sway.app().arg("--restore").output()?;
        ensure!(
            result.status.success(),
            "workspace restore: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let live = sway.snapshot()?;
        ensure!(
            live.workspaces
                .iter()
                .any(|w| w.name == workspace_name && w.output == target),
            "Workspace did not move"
        );
        ensure!(
            gui.try_wait()?.is_none(),
            "Window died during workspace move"
        );
        println!("✓ restore moves an existing workspace and its live Wayland window");

        Ok(())
    })();
    let _ = gui.kill();
    let _ = gui.wait();
    gui_result?;

    let before_timeout = sway.snapshot()?;
    let mut timeout_candidate = before_timeout.clone();
    timeout_candidate.outputs[1].rect.x = 1500;
    let (mut guard, mut reply) = sway.guard(&before_timeout, &timeout_candidate)?;
    sway.apply_outputs(&timeout_candidate)?;
    // Keep stdin open: prove the independent 20-second deadline, not merely EOF.
    let started = Instant::now();
    let mut line = String::new();
    reply.read_line(&mut line)?;
    ensure!(line.trim() == "reverted", "Timeout rollback: {line}");
    ensure!(
        started.elapsed() >= Duration::from_secs(18),
        "Deadline fired too early"
    );
    guard.wait()?;
    equivalent_outputs(&before_timeout, &sway.snapshot()?);
    println!("✓ unconfirmed layout reverts after the real 20-second timeout");
    Ok(())
}
