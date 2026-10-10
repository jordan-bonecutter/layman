use anyhow::Result;
use serde::Deserialize;
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Geometry {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Node {
    #[serde(default)]
    pub id: u64,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub window: Option<u64>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub rect: Geometry,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub floating_nodes: Vec<Node>,
}
#[derive(Clone, Debug)]
pub struct Window {
    pub title: String,
    pub rect: Geometry,
    pub focused: bool,
    pub floating: bool,
}
#[derive(Clone, Debug)]
pub struct Workspace {
    pub output: String,
    pub name: String,
    pub rect: Geometry,
    pub windows: Vec<Window>,
}
pub fn workspaces(tree: &Node) -> Vec<Workspace> {
    fn windows(n: &Node, floating: bool, list: &mut Vec<Window>) {
        if n.app_id.is_some() || n.window.is_some() {
            list.push(Window {
                title: n.app_id.clone().or(n.name.clone()).unwrap_or_default(),
                rect: n.rect.clone(),
                focused: n.focused,
                floating,
            });
        }
        for child in &n.nodes {
            windows(child, floating, list);
        }
        for child in &n.floating_nodes {
            windows(child, true, list);
        }
    }
    fn walk(n: &Node, output: &str, list: &mut Vec<Workspace>) {
        let output = if n.kind == "output" {
            n.name.as_deref().unwrap_or(output)
        } else {
            output
        };
        if n.kind == "workspace" && n.name.as_deref() != Some("__i3_scratch") {
            let mut entries = Vec::new();
            windows(n, false, &mut entries);
            list.push(Workspace {
                output: output.into(),
                name: n.name.clone().unwrap_or_default(),
                rect: n.rect.clone(),
                windows: entries,
            });
        } else {
            for child in &n.nodes {
                walk(child, output, list);
            }
        }
    }
    let mut list = Vec::new();
    walk(tree, "", &mut list);
    list
}
pub fn poll(ctx: eframe::egui::Context) -> mpsc::Receiver<Result<Vec<Workspace>>> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        loop {
            let result = crate::ipc::tree().map(|tree| workspaces(&tree));
            match send.try_send(result) {
                Ok(()) => ctx.request_repaint(),
                Err(mpsc::TrySendError::Disconnected(_)) => break,
                Err(mpsc::TrySendError::Full(_)) => {}
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    receive
}
