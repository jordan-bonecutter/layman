use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
}
impl Mode {
    pub fn label(&self) -> String {
        if self.refresh == 0 {
            format!("{} × {}  ·  default refresh", self.width, self.height)
        } else {
            format!(
                "{} × {}  ·  {:.2} Hz",
                self.width,
                self.height,
                self.refresh as f64 / 1000.
            )
        }
    }

    pub fn command(&self) -> String {
        if self.refresh == 0 {
            format!("{}x{}", self.width, self.height)
        } else {
            format!(
                "{}x{}@{:.3}Hz",
                self.width,
                self.height,
                self.refresh as f64 / 1000.
            )
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub name: String,
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default = "normal")]
    pub transform: String,
    #[serde(default)]
    pub rect: Rect,
    #[serde(default)]
    pub current_mode: Option<Mode>,
    #[serde(default)]
    pub modes: Vec<Mode>,
    #[serde(default)]
    pub non_desktop: bool,
}
fn one() -> f64 {
    1.
}
fn normal() -> String {
    "normal".into()
}
pub const TRANSFORMS: [&str; 8] = [
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];
impl Output {
    pub fn size(&self) -> (i32, i32) {
        let mode = self.current_mode.as_ref().or(self.modes.first());
        let (mut w, mut h) = mode.map(|m| (m.width, m.height)).unwrap_or((1920, 1080));
        if matches!(
            self.transform.as_str(),
            "90" | "270" | "flipped-90" | "flipped-270"
        ) {
            std::mem::swap(&mut w, &mut h);
        }
        (
            (w as f64 / self.scale).round() as i32,
            (h as f64 / self.scale).round() as i32,
        )
    }
    pub fn identity(&self) -> String {
        if !self.make.is_empty()
            && !self.model.is_empty()
            && !self.serial.is_empty()
            && self.serial != "Unknown"
        {
            format!("{} {} {}", self.make, self.model, self.serial)
        } else {
            self.name.clone()
        }
    }
    pub fn same_device(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub output: String,
    #[serde(default)]
    pub focused: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    pub outputs: Vec<Output>,
    pub workspaces: Vec<Workspace>,
}
impl Layout {
    pub fn new(outputs: Vec<Output>, workspaces: Vec<Workspace>) -> Self {
        Self {
            version: 1,
            outputs,
            workspaces,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("Unsupported layout version {}", self.version);
        }
        if !self.outputs.iter().any(|o| o.active) {
            bail!("Keep at least one display enabled.");
        }
        let mut names = HashSet::new();
        for o in &self.outputs {
            quote(&o.name)?;
            quote(&o.identity())?;
            if !names.insert(&o.name) {
                bail!("Duplicate output {}", o.name);
            }
            if !o.scale.is_finite() || !(0.5..=4.).contains(&o.scale) {
                bail!("Scale must be between 0.5 and 4.");
            }
            if !TRANSFORMS.contains(&o.transform.as_str()) {
                bail!("Invalid transform.");
            }
            if o.rect.x.abs_diff(0) > 100_000 || o.rect.y.abs_diff(0) > 100_000 {
                bail!("Display position is out of range.");
            }
            if let Some(m) = &o.current_mode {
                if m.width <= 0
                    || m.height <= 0
                    || m.refresh < 0
                    || m.width > 32768
                    || m.height > 32768
                {
                    bail!("Invalid display mode.");
                }
            } else if o.active {
                bail!("Choose a mode for {}.", o.name);
            }
        }
        let mut workspaces = HashSet::new();
        for w in &self.workspaces {
            quote(&w.name)?;
            if !workspaces.insert(&w.name) {
                bail!("Duplicate workspace {}", w.name);
            }
            if !self.outputs.iter().any(|o| o.name == w.output && o.active) {
                bail!("Workspace {} needs an enabled display.", w.name);
            }
        }
        Ok(())
    }
    pub fn output_commands(&self, stable: bool) -> Result<Vec<String>> {
        self.validate()?;
        // Enable first: never turn off the last display before its replacement is ready.
        let mut commands = Vec::new();
        for o in self.outputs.iter().filter(|o| o.active) {
            let target = if stable { o.identity() } else { o.name.clone() };
            commands.push(format!(
                "output {} enable mode {} scale {:.4} transform {} pos {} {}",
                quote(&target)?,
                o.current_mode.as_ref().unwrap().command(),
                o.scale,
                o.transform,
                o.rect.x,
                o.rect.y
            ));
        }
        for o in self.outputs.iter().filter(|o| !o.active) {
            commands.push(format!(
                "output {} disable",
                quote(&if stable { o.identity() } else { o.name.clone() })?
            ));
        }
        Ok(commands)
    }
    /// Resolve saved identities against connected outputs. Unknown outputs stay untouched.
    pub fn connected(&self, live: &Self) -> Result<Self> {
        self.validate()?;
        let mut outputs = Vec::new();
        let mut workspaces = Vec::new();
        for saved in &self.outputs {
            if let Some(current) = live.outputs.iter().find(|o| o.same_device(saved)) {
                let mut o = saved.clone();
                o.name = current.name.clone();
                o.modes = current.modes.clone();
                // A removed/custom mode must not silently be attempted on restore.
                if o.active
                    && !o.modes.contains(o.current_mode.as_ref().unwrap())
                    && o.current_mode != current.current_mode
                {
                    bail!(
                        "Saved mode for {} is no longer advertised. Refresh and select a mode.",
                        o.name
                    );
                }
                for w in self
                    .workspaces
                    .iter()
                    .filter(|w| w.output == saved.name && o.active)
                {
                    let mut w = w.clone();
                    w.output = o.name.clone();
                    workspaces.push(w);
                }
                outputs.push(o);
            }
        }
        // Do not disable the only remaining connected screen after undocking.
        if !outputs.iter().any(|o| o.active) {
            bail!("No enabled saved display is connected; leaving Sway unchanged.");
        }
        Ok(Self::new(outputs, workspaces))
    }
    pub fn horizontal(&mut self) {
        let mut x = 0;
        for o in self.outputs.iter_mut().filter(|o| o.active) {
            o.rect.x = x;
            o.rect.y = 0;
            x += o.size().0;
        }
    }
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let active: Vec<_> = self.outputs.iter().filter(|o| o.active).collect();
        for (i, a) in active.iter().enumerate() {
            for b in &active[i + 1..] {
                let (aw, ah) = a.size();
                let (bw, bh) = b.size();
                if a.rect.x < b.rect.x + bw
                    && b.rect.x < a.rect.x + aw
                    && a.rect.y < b.rect.y + bh
                    && b.rect.y < a.rect.y + ah
                {
                    warnings.push(format!("{} and {} overlap.", a.name, b.name));
                }
            }
        }
        if active.len() > 1 {
            let mut reached = HashSet::from([0]);
            loop {
                let before = reached.len();
                for (i, a) in active.iter().enumerate() {
                    if !reached.contains(&i) {
                        continue;
                    }
                    let (aw, ah) = a.size();
                    for (j, b) in active.iter().enumerate() {
                        let (bw, bh) = b.size();
                        let vertical = (a.rect.x + aw == b.rect.x || b.rect.x + bw == a.rect.x)
                            && a.rect.y < b.rect.y + bh
                            && b.rect.y < a.rect.y + ah;
                        let horizontal = (a.rect.y + ah == b.rect.y || b.rect.y + bh == a.rect.y)
                            && a.rect.x < b.rect.x + bw
                            && b.rect.x < a.rect.x + aw;
                        if vertical || horizontal {
                            reached.insert(j);
                        }
                    }
                }
                if reached.len() == before {
                    break;
                }
            }
            if reached.len() != active.len() {
                warnings.push(
                    "Some displays do not touch. The pointer may not cross between them.".into(),
                );
            }
        }
        warnings
    }
}
/// Sway config and IPC share a parser. Reject metacharacters instead of trusting escaping.
pub fn quote(value: &str) -> Result<String> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_control() || "\"'\\;,$`{}[]".contains(c))
    {
        bail!("Unsupported characters in Sway name: {value:?}");
    }
    Ok(format!("\"{value}\""))
}
pub fn snap(layout: &Layout, index: usize, x: i32, y: i32, threshold: i32) -> (i32, i32) {
    let (w, h) = layout.outputs[index].size();
    let mut best_x = (threshold + 1, x);
    let mut best_y = (threshold + 1, y);
    for (i, o) in layout
        .outputs
        .iter()
        .enumerate()
        .filter(|(i, o)| *i != index && o.active)
    {
        let _ = i;
        let (ow, oh) = o.size();
        for candidate in [o.rect.x - w, o.rect.x, o.rect.x + ow - w, o.rect.x + ow] {
            let distance = (candidate - x).abs();
            if distance <= threshold && distance < best_x.0 {
                best_x = (distance, candidate);
            }
        }
        for candidate in [o.rect.y - h, o.rect.y, o.rect.y + oh - h, o.rect.y + oh] {
            let distance = (candidate - y).abs();
            if distance <= threshold && distance < best_y.0 {
                best_y = (distance, candidate);
            }
        }
    }
    (best_x.1, best_y.1)
}
