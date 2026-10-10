use crate::model::{Layout, Output, Workspace, quote};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

pub fn request_at<T: DeserializeOwned>(
    socket: &std::path::Path,
    kind: u32,
    payload: &str,
) -> Result<T> {
    let mut stream = UnixStream::connect(socket).context("Cannot connect to Sway IPC")?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"i3-ipc")?;
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(&kind.to_le_bytes())?;
    stream.write_all(payload.as_bytes())?;
    let mut header = [0; 14];
    stream.read_exact(&mut header)?;
    if &header[..6] != b"i3-ipc" {
        bail!("Invalid Sway IPC header");
    }
    let len = u32::from_le_bytes(header[6..10].try_into()?) as usize;
    let response_kind = u32::from_le_bytes(header[10..14].try_into()?);
    if response_kind != kind || len > 16 * 1024 * 1024 {
        bail!("Invalid Sway IPC response");
    }
    let mut data = vec![0; len];
    stream.read_exact(&mut data)?;
    serde_json::from_slice(&data).context("Invalid Sway JSON")
}
fn request<T: DeserializeOwned>(kind: u32, payload: &str) -> Result<T> {
    let socket = PathBuf::from(
        std::env::var_os("SWAYSOCK")
            .context("SWAYSOCK is unset. Run Layman inside Sway, or use --demo.")?,
    );
    request_at(&socket, kind, payload)
}
#[derive(Deserialize)]
struct Reply {
    success: bool,
    #[serde(default)]
    error: Option<String>,
}
pub fn command(command: &str) -> Result<()> {
    let replies: Vec<Reply> = request(0, command)?;
    if replies.is_empty() {
        bail!("Empty command reply from Sway");
    }
    for r in replies {
        if !r.success {
            bail!("Sway rejected {command}: {}", r.error.unwrap_or_default());
        }
    }
    Ok(())
}
pub fn snapshot() -> Result<Layout> {
    let mut outputs: Vec<Output> = request(3, "")?;
    outputs.retain(|o| !o.non_desktop);
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    for o in &mut outputs {
        o.modes.dedup();
        if let Some(mode) = &o.current_mode
            && !o.modes.contains(mode)
        {
            o.modes.insert(0, mode.clone());
        }
        if o.current_mode.is_none() {
            o.current_mode = o.modes.first().cloned();
        }
    }
    let workspaces: Vec<Workspace> = request(1, "")?;
    let layout = Layout::new(outputs, workspaces);
    layout.validate()?;
    Ok(layout)
}
pub fn apply(layout: &Layout) -> Result<()> {
    layout.validate()?;
    let live = snapshot()?;
    let resolved = layout.connected(&live)?;
    for c in resolved.output_commands(false)? {
        command(&c)?;
    }
    move_workspaces(&resolved)
}
fn move_workspaces(layout: &Layout) -> Result<()> {
    let live: Vec<Workspace> = request(1, "")?;
    let focused = live.iter().find(|w| w.focused).map(|w| w.name.clone());
    let result = (|| {
        for w in &layout.workspaces {
            if let Some(current) = live.iter().find(|current| current.name == w.name) {
                if current.output == w.output {
                    continue;
                }
                command(&format!(
                    "workspace --no-auto-back-and-forth {}",
                    quote(&w.name)?
                ))?;
                command(&format!("move workspace to output {}", quote(&w.output)?))?;
            }
        }
        Ok(())
    })();
    if let Some(name) = focused {
        command(&format!(
            "workspace --no-auto-back-and-forth {}",
            quote(&name)?
        ))?;
    }
    result
}

pub fn config_path() -> Result<PathBuf> {
    #[derive(Deserialize)]
    struct Version {
        loaded_config_file_name: PathBuf,
    }
    Ok(request::<Version>(7, "")?.loaded_config_file_name)
}
