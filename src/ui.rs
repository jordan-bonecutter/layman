use crate::{
    icons::{self, Icon},
    integration, ipc, live,
    model::{Layout, Mode, Output, Rect as OutputRect, Workspace, snap},
    preview::Preview,
    store,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

const BG: Color32 = Color32::from_rgb(17, 21, 27);
const PANEL: Color32 = Color32::from_rgb(24, 29, 37);
const LINE: Color32 = Color32::from_rgb(46, 55, 67);
const MUTED: Color32 = Color32::from_rgb(147, 160, 177);
const MINT: Color32 = Color32::from_rgb(150, 235, 200);
const TEXT: Color32 = Color32::from_rgb(233, 239, 246);
const COLORS: [Color32; 4] = [
    MINT,
    Color32::from_rgb(169, 185, 255),
    Color32::from_rgb(247, 198, 132),
    Color32::from_rgb(235, 163, 194),
];

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    x: i32,
    y: i32,
    scale: f32,
    origin: Pos2,
    pointer_start: Pos2,
}

pub struct App {
    layout: Layout,
    baseline: Layout,
    selected: usize,
    tab: usize,
    demo: bool,
    status: String,
    preview: Option<Preview>,
    snapping: bool,
    zoom: f32,
    pan: Vec2,
    drag: Option<Drag>,
    help: bool,
    new_workspace: String,
    scenes: Vec<live::Workspace>,
    receiver: Option<std::sync::mpsc::Receiver<anyhow::Result<Vec<live::Workspace>>>>,
    live_error: Option<String>,
    config_patch: Option<integration::Patch>,
    config_message: String,
}
impl App {
    pub fn new(ctx: &egui::Context, demo: bool) -> anyhow::Result<Self> {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = Color32::from_rgb(48, 82, 73);
        visuals.selection.stroke = Stroke::new(1_f32, MINT);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 42, 52);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1_f32, LINE);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 59, 70);
        visuals.widgets.active.bg_fill = Color32::from_rgb(55, 82, 76);
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10., 10.);
            s.spacing.button_padding = Vec2::new(12., 8.);
            s.text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(14.));
            s.text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(14.));
        });
        let layout = if demo {
            demo_layout()
        } else {
            ipc::snapshot()?
        };
        Ok(Self {
            baseline: layout.clone(),
            layout,
            selected: 0,
            tab: 0,
            demo,
            status: String::new(),
            preview: None,
            snapping: true,
            zoom: 1.,
            pan: Vec2::ZERO,
            drag: None,
            help: false,
            new_workspace: String::new(),
            scenes: if demo { demo_scenes() } else { Vec::new() },
            receiver: if demo {
                None
            } else {
                Some(live::poll(ctx.clone()))
            },
            live_error: None,
            config_patch: None,
            config_message: String::new(),
        })
    }
    fn message(&mut self, result: anyhow::Result<()>, success: &str) {
        self.status = match result {
            Ok(()) => success.into(),
            Err(e) => format!("{e:#}"),
        };
    }
    fn refresh(&mut self) {
        match ipc::snapshot() {
            Ok(l) => {
                self.layout = l.clone();
                self.baseline = l;
                self.selected = 0;
                self.fit();
                self.status.clear();
            }
            Err(e) => self.status = format!("{e:#}"),
        }
    }
    fn load_saved(&mut self) {
        let result = (|| {
            let saved = store::load()?.ok_or_else(|| {
                anyhow::anyhow!("No saved layout yet. Preview a layout and choose Keep.")
            })?;
            // Keep unknown connected displays visible/editable in the draft.
            let mut resolved = saved.connected(&self.baseline)?;
            for o in &self.baseline.outputs {
                if !resolved.outputs.iter().any(|r| r.name == o.name) {
                    resolved.outputs.push(o.clone());
                }
            }
            self.layout = resolved;
            self.selected = 0;
            self.fit();
            Ok(())
        })();
        self.message(result, "");
    }
    fn fit(&mut self) {
        self.zoom = 1.;
        self.pan = Vec2::ZERO;
    }
    fn preview(&mut self) {
        match Preview::start(&self.layout) {
            Ok(p) => {
                self.preview = Some(p);
                self.status.clear();
            }
            Err(e) => self.status = format!("{e:#}"),
        }
    }
    fn finish(&mut self, keep: bool) {
        if let Some(mut preview) = self.preview.take() {
            let result = preview.finish(keep);
            if result.is_ok() && keep {
                self.baseline = self.layout.clone();
                self.help = true;
            }
            self.message(result, "");
            if keep && self.help {
                self.check_config();
            }
        }
    }
    fn check_config(&mut self) {
        self.config_patch = None;
        match integration::check() {
            Ok(patch) => {
                self.config_message = if patch.installed() {
                    "Include already installed.".into()
                } else {
                    String::new()
                };
                self.config_patch = Some(patch);
            }
            Err(e) => self.config_message = format!("{e:#}"),
        }
    }
    fn read_live(&mut self) {
        if let Some(receiver) = &self.receiver {
            while let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(scenes) => {
                        for scene in &scenes {
                            if !self.layout.workspaces.iter().any(|w| w.name == scene.name)
                                && self
                                    .layout
                                    .outputs
                                    .iter()
                                    .any(|o| o.name == scene.output && o.active)
                            {
                                let workspace = Workspace {
                                    name: scene.name.clone(),
                                    output: scene.output.clone(),
                                    focused: false,
                                };
                                self.layout.workspaces.push(workspace.clone());
                                self.baseline.workspaces.push(workspace);
                            }
                        }
                        self.scenes = scenes;
                        self.live_error = None;
                    }
                    Err(e) => self.live_error = Some(format!("Live previews unavailable: {e:#}")),
                }
            }
        }
    }
    pub fn draw(&mut self, ctx: &egui::Context) {
        self.read_live();
        if self.preview.as_ref().is_some_and(|p| p.remaining() == 0) {
            self.finish(false);
        }
        if self.preview.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::new().fill(BG).inner_margin(16.))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.preview.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        let (logo, _) = ui.allocate_exact_size(Vec2::new(26., 36.), Sense::hover());
                        let screen = Rect::from_min_size(
                            logo.left_top() + Vec2::new(0., 8.),
                            Vec2::new(18., 14.),
                        );
                        ui.painter().rect_stroke(
                            screen,
                            3.,
                            Stroke::new(1.5_f32, MINT),
                            StrokeKind::Inside,
                        );
                        ui.painter().rect_stroke(
                            screen.translate(Vec2::new(6., 6.)),
                            3.,
                            Stroke::new(1.5_f32, MINT),
                            StrokeKind::Inside,
                        );
                        ui.add_space(14.);
                        if icons::button(ui, Icon::Row, "Arrange in a row", false).clicked() {
                            self.layout.horizontal();
                            self.fit();
                        }
                        if icons::button(ui, Icon::Fit, "Fit displays", false).clicked() {
                            self.fit();
                        }
                        if icons::button(
                            ui,
                            Icon::Snap,
                            "Snap display edges · Alt bypasses",
                            self.snapping,
                        )
                        .clicked()
                        {
                            self.snapping = !self.snapping;
                        }
                        ui.add_space(10.);
                        if icons::button(ui, Icon::Minus, "Zoom out", false).clicked() {
                            self.zoom = (self.zoom / 1.2).max(0.3);
                        }
                        if icons::button(
                            ui,
                            Icon::Zoom,
                            &format!("Reset zoom · {:.0}%", self.zoom * 100.),
                            false,
                        )
                        .clicked()
                        {
                            self.fit();
                        }
                        if icons::button(ui, Icon::Plus, "Zoom in", false).clicked() {
                            self.zoom = (self.zoom * 1.2).min(4.);
                        }
                        ui.add_space(10.);
                        ui.add_enabled_ui(!self.demo, |ui| {
                            if icons::button(ui, Icon::Refresh, "Refresh displays from Sway", false)
                                .clicked()
                            {
                                self.refresh();
                            }
                        });
                    });
                });
            });
        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::new().fill(BG).inner_margin(16.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(p) = &self.preview {
                        ui.label(
                            RichText::new(format!("{}s", p.remaining()))
                                .monospace()
                                .color(MINT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(ui, Icon::Save, "Keep & save", true).clicked() {
                                self.finish(true);
                            }
                            if icons::button(ui, Icon::Revert, "Revert preview", false).clicked() {
                                self.finish(false);
                            }
                        });
                    } else {
                        if !self.status.is_empty() {
                            ui.label(RichText::new(&self.status).size(12.).color(COLORS[2]));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(!self.demo, |ui| {
                                if icons::button(ui, Icon::Preview, "Preview layout", true)
                                    .clicked()
                                {
                                    self.preview();
                                }
                            });
                            if icons::button(ui, Icon::Reset, "Reset edits", false).clicked() {
                                self.layout = self.baseline.clone();
                                self.selected = 0;
                                self.fit();
                                self.status.clear();
                            }
                            ui.add_enabled_ui(!self.demo, |ui| {
                                if icons::button(ui, Icon::Load, "Load saved layout", false)
                                    .clicked()
                                {
                                    self.load_saved();
                                }
                            });
                        });
                    }
                });
            });
        egui::SidePanel::right("inspector")
            .exact_width(292.)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(22.))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.preview.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.tab, 0, "Display");
                        ui.selectable_value(&mut self.tab, 1, "Workspaces");
                    });
                    ui.add_space(14.);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.tab == 0 {
                            self.inspector(ui);
                        } else {
                            self.workspaces(ui);
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(24.))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.preview.is_none(), |ui| {
                    for warning in self.layout.warnings() {
                        ui.label(RichText::new(warning).size(12.).color(COLORS[2]));
                    }
                    if let Some(error) = &self.live_error {
                        ui.label(RichText::new(error).size(12.).color(COLORS[2]));
                    }
                    self.canvas(ui);
                });
            });
        if self.help {
            self.save_dialog(ctx);
        }
    }
    fn save_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.help;
        egui::Window::new("Saved layout")
            .collapsible(false)
            .resizable(true)
            .default_width(590.)
            .open(&mut open)
            .show(ctx, |ui| {
                let line = store::include_line().unwrap_or_else(|e| e.to_string());
                ui.label(
                    "Include this at the end of your Sway config to restore the layout on reload.",
                );
                ui.horizontal(|ui| {
                    ui.code(&line);
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(line.clone());
                    }
                });
                if ui.button("Check Sway config").clicked() {
                    self.check_config();
                }
                if let Some(patch) = &self.config_patch {
                    ui.label(
                        RichText::new(patch.path.display().to_string())
                            .size(12.)
                            .color(MUTED),
                    );
                    if !patch.installed() {
                        egui::ScrollArea::both().max_height(260.).show(ui, |ui| {
                            egui::Frame::new()
                                .fill(BG)
                                .inner_margin(12.)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.;
                                    for (kind, line) in patch.diff() {
                                        let color = match kind {
                                            '+' => MINT,
                                            '-' => Color32::from_rgb(246, 153, 153),
                                            _ => MUTED,
                                        };
                                        ui.label(
                                            RichText::new(format!("{kind}{line}"))
                                                .monospace()
                                                .size(12.)
                                                .color(color),
                                        );
                                    }
                                });
                        });
                        if ui.button("Apply diff").clicked() {
                            match patch.apply() {
                                Ok(()) => self.check_config(),
                                Err(e) => self.config_message = format!("{e:#}"),
                            }
                        }
                    }
                }
                if !self.config_message.is_empty() {
                    ui.label(&self.config_message);
                }
            });
        self.help = open;
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("DISPLAYS").size(11.).color(MUTED));
        for (i, o) in self.layout.outputs.iter().enumerate() {
            let color = COLORS[i % COLORS.len()];
            let label = format!("{}   {}", if o.active { "[on]" } else { "[off]" }, o.name);
            if ui
                .selectable_label(self.selected == i, RichText::new(label).color(color))
                .clicked()
            {
                self.selected = i;
            }
        }
        ui.add_space(16.);
        ui.separator();
        ui.add_space(12.);
        let Some(o) = self.layout.outputs.get_mut(self.selected) else {
            return;
        };
        ui.label(RichText::new(&o.name).size(23.).strong());
        ui.label(
            RichText::new(format!("{} {}", o.make, o.model))
                .size(12.)
                .color(MUTED),
        );
        ui.add_space(6.);
        ui.checkbox(&mut o.active, "Display enabled");
        ui.add_space(8.);
        ui.label(RichText::new("Resolution & refresh").size(12.).color(MUTED));
        egui::ComboBox::from_id_salt("mode")
            .width(235.)
            .selected_text(
                o.current_mode
                    .as_ref()
                    .map(Mode::label)
                    .unwrap_or_else(|| "Select mode".into()),
            )
            .show_ui(ui, |ui| {
                for m in &o.modes {
                    ui.selectable_value(&mut o.current_mode, Some(m.clone()), m.label());
                }
            });
        ui.add_space(8.);
        icons::scale(ui, &mut o.scale);
        ui.add_space(12.);
        ui.label(RichText::new("Orientation").size(12.).color(MUTED));
        let flipped = o.transform.starts_with("flipped");
        let angle = o
            .transform
            .trim_start_matches("flipped")
            .trim_start_matches('-')
            .parse::<u16>()
            .unwrap_or(0);
        ui.horizontal(|ui| {
            for rotation in [0, 90, 180, 270] {
                if icons::button(
                    ui,
                    Icon::Rotation(rotation),
                    &format!("Rotate {rotation}°"),
                    angle == rotation,
                )
                .clicked()
                {
                    o.transform = transform(rotation, flipped);
                }
            }
        });
        if icons::button(ui, Icon::Flip, "Mirror display", flipped).clicked() {
            o.transform = transform(angle, !flipped);
        }
        if let Err(e) = self.layout.validate() {
            ui.add_space(12.);
            ui.label(RichText::new(e.to_string()).color(COLORS[2]));
        }
    }
    fn workspaces(&mut self, ui: &mut egui::Ui) {
        for (i, w) in self.layout.workspaces.iter_mut().enumerate() {
            ui.label(RichText::new(format!("Workspace {}", w.name)).strong());
            egui::ComboBox::from_id_salt(("workspace", i))
                .width(235.)
                .selected_text(&w.output)
                .show_ui(ui, |ui| {
                    for o in self.layout.outputs.iter().filter(|o| o.active) {
                        ui.selectable_value(&mut w.output, o.name.clone(), &o.name);
                    }
                });
            ui.add_space(6.);
        }
        ui.separator();
        ui.label(
            RichText::new("Add a future workspace")
                .size(12.)
                .color(MUTED),
        );
        ui.text_edit_singleline(&mut self.new_workspace);
        if ui.button("Add workspace").clicked() {
            let name = self.new_workspace.trim().to_string();
            if !name.is_empty()
                && !self.layout.workspaces.iter().any(|w| w.name == name)
                && let Some(o) = self.layout.outputs.iter().find(|o| o.active)
            {
                self.layout.workspaces.push(Workspace {
                    name,
                    output: o.name.clone(),
                    focused: false,
                });
                self.new_workspace.clear();
            }
        }
    }
    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(Vec2::new(200., 240.)),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 16., Color32::from_rgb(20, 25, 32));
        painter.rect_stroke(rect, 16., Stroke::new(1_f32, LINE), StrokeKind::Inside);
        let active: Vec<_> = self
            .layout
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.active)
            .collect();
        if active.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Enable a display in the inspector",
                FontId::proportional(16.),
                MUTED,
            );
            return;
        }
        let min_x = active.iter().map(|(_, o)| o.rect.x).min().unwrap() as f32;
        let min_y = active.iter().map(|(_, o)| o.rect.y).min().unwrap() as f32;
        let max_x = active
            .iter()
            .map(|(_, o)| o.rect.x + o.size().0)
            .max()
            .unwrap() as f32;
        let max_y = active
            .iter()
            .map(|(_, o)| o.rect.y + o.size().1)
            .max()
            .unwrap() as f32;
        let base = ((rect.width() - 110.) / (max_x - min_x).max(1.))
            .min((rect.height() - 110.) / (max_y - min_y).max(1.))
            .max(0.005);
        let scale = self.drag.map(|d| d.scale).unwrap_or(base * self.zoom);
        let origin = self.drag.map(|d| d.origin).unwrap_or(
            rect.center() - Vec2::new((max_x + min_x) * scale / 2., (max_y + min_y) * scale / 2.)
                + self.pan,
        );
        let mut start_drag = None;
        for (i, o) in active {
            let (w, h) = o.size();
            let r = Rect::from_min_size(
                origin + Vec2::new(o.rect.x as f32, o.rect.y as f32) * scale,
                Vec2::new(w as f32, h as f32) * scale,
            );
            let color = COLORS[i % COLORS.len()];
            let selected = self.selected == i;
            let interaction = ui.interact(
                r.intersect(rect),
                ui.id().with(("monitor", i)),
                Sense::click_and_drag(),
            );
            if interaction.clicked() {
                self.selected = i;
            }
            if interaction.drag_started() {
                self.selected = i;
                start_drag = Some(Drag {
                    index: i,
                    x: o.rect.x,
                    y: o.rect.y,
                    scale,
                    origin,
                    pointer_start: ui
                        .input(|input| input.pointer.press_origin().unwrap_or(r.center())),
                });
            }
            if interaction.hovered() || interaction.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            painter.rect_filled(
                r.translate(Vec2::new(0., 6.)),
                10.,
                Color32::from_black_alpha(70),
            );
            painter.rect_filled(
                r,
                10.,
                if selected {
                    Color32::from_rgb(37, 57, 53)
                } else {
                    Color32::from_rgb(32, 40, 51)
                },
            );
            painter.rect_stroke(
                r,
                10.,
                Stroke::new(
                    if selected { 2_f32 } else { 1_f32 },
                    if selected { color } else { LINE },
                ),
                StrokeKind::Inside,
            );
            let inner = r.shrink(14.);
            if r.width() > 100. && r.height() > 90. {
                painter.text(
                    inner.left_top(),
                    Align2::LEFT_TOP,
                    &o.name,
                    FontId::proportional(14.),
                    color,
                );
                painter.text(
                    inner.right_top(),
                    Align2::RIGHT_TOP,
                    format!("{w} × {h}"),
                    FontId::proportional(10.),
                    MUTED,
                );
                let assigned: Vec<_> = self
                    .layout
                    .workspaces
                    .iter()
                    .filter(|w| w.output == o.name)
                    .collect();
                let body = Rect::from_min_max(inner.min + Vec2::new(0., 28.), inner.max);
                paint_workspaces(&painter, body, &assigned, &self.scenes, color);
            } else {
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    &o.name,
                    FontId::proportional(12.),
                    color,
                );
            }
        }
        if let Some(d) = start_drag {
            self.drag = Some(d);
        }
        if let Some(Drag {
            index: i,
            x,
            y,
            scale,
            pointer_start,
            ..
        }) = self.drag
        {
            let delta = ui.input(|input| {
                input
                    .pointer
                    .interact_pos()
                    .map(|position| position - pointer_start)
                    .unwrap_or_else(|| {
                        Vec2::new(
                            (self.layout.outputs[i].rect.x - x) as f32 * scale,
                            (self.layout.outputs[i].rect.y - y) as f32 * scale,
                        )
                    })
            });
            let new_x = (x + (delta.x / scale).round() as i32).clamp(-100000, 100000);
            let new_y = (y + (delta.y / scale).round() as i32).clamp(-100000, 100000);
            let (new_x, new_y) = if self.snapping && !ui.input(|i| i.modifiers.alt) {
                snap(&self.layout, i, new_x, new_y, (12. / scale) as i32)
            } else {
                (new_x, new_y)
            };
            self.layout.outputs[i].rect = OutputRect { x: new_x, y: new_y };
            if !ui.input(|i| i.pointer.primary_down()) {
                self.drag = None;
            }
        } else if response.dragged_by(egui::PointerButton::Middle) {
            self.pan += response.drag_delta();
        }
        if response.hovered() && !ui.ctx().wants_keyboard_input() {
            let (dx, dy) = ui.input(|i| {
                let step = if i.modifiers.shift { 10 } else { 1 };
                (
                    (i.key_pressed(egui::Key::ArrowRight) as i32
                        - i.key_pressed(egui::Key::ArrowLeft) as i32)
                        * step,
                    (i.key_pressed(egui::Key::ArrowDown) as i32
                        - i.key_pressed(egui::Key::ArrowUp) as i32)
                        * step,
                )
            });
            if let Some(o) = self.layout.outputs.get_mut(self.selected) {
                o.rect.x = (o.rect.x + dx).clamp(-100000, 100000);
                o.rect.y = (o.rect.y + dy).clamp(-100000, 100000);
            }
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
    }
}
pub fn demo_layout() -> Layout {
    let modes = vec![
        Mode {
            width: 2560,
            height: 1440,
            refresh: 144000,
        },
        Mode {
            width: 2560,
            height: 1440,
            refresh: 60000,
        },
        Mode {
            width: 1920,
            height: 1080,
            refresh: 60000,
        },
    ];
    let desktop = Output {
        name: "DP-1".into(),
        make: "Dell".into(),
        model: "Studio display".into(),
        serial: "demo-1".into(),
        active: true,
        scale: 1.,
        transform: "normal".into(),
        rect: OutputRect { x: 0, y: 0 },
        current_mode: Some(modes[0].clone()),
        modes,
        non_desktop: false,
    };
    let laptop_mode = Mode {
        width: 1920,
        height: 1200,
        refresh: 60000,
    };
    let laptop = Output {
        name: "eDP-1".into(),
        make: "Framework".into(),
        model: "Built-in display".into(),
        serial: "Unknown".into(),
        active: true,
        scale: 1.25,
        transform: "normal".into(),
        rect: OutputRect { x: 2560, y: 480 },
        current_mode: Some(laptop_mode.clone()),
        modes: vec![laptop_mode],
        non_desktop: false,
    };
    Layout::new(
        vec![desktop, laptop],
        vec![
            Workspace {
                name: "1: code".into(),
                output: "DP-1".into(),
                focused: true,
            },
            Workspace {
                name: "2: web".into(),
                output: "DP-1".into(),
                focused: false,
            },
            Workspace {
                name: "3: chat".into(),
                output: "eDP-1".into(),
                focused: false,
            },
        ],
    )
}

pub fn transform(angle: u16, flipped: bool) -> String {
    match (angle, flipped) {
        (0, false) => "normal".into(),
        (0, true) => "flipped".into(),
        (a, true) => format!("flipped-{a}"),
        (a, false) => a.to_string(),
    }
}
fn paint_workspaces(
    p: &egui::Painter,
    area: Rect,
    assigned: &[&Workspace],
    scenes: &[live::Workspace],
    color: Color32,
) {
    if assigned.is_empty() {
        return;
    }
    let columns = (assigned.len() as f32).sqrt().ceil() as usize;
    let rows = assigned.len().div_ceil(columns);
    let gap = 8.;
    let size = Vec2::new(
        (area.width() - gap * (columns - 1) as f32) / columns as f32,
        (area.height() - gap * (rows - 1) as f32) / rows as f32,
    );
    if size.x < 6. || size.y < 6. {
        return;
    }
    for (i, w) in assigned.iter().enumerate() {
        let tile = Rect::from_min_size(
            area.min
                + Vec2::new(
                    (i % columns) as f32 * (size.x + gap),
                    (i / columns) as f32 * (size.y + gap),
                ),
            size,
        );
        p.rect_filled(tile, 4., BG);
        p.rect_stroke(tile, 4., Stroke::new(1_f32, LINE), StrokeKind::Inside);
        let clip = p.with_clip_rect(tile.shrink(2.));
        clip.text(
            tile.min + Vec2::splat(6.),
            Align2::LEFT_TOP,
            &w.name,
            FontId::proportional(10.),
            color,
        );
        let viewport =
            Rect::from_min_max(tile.min + Vec2::new(5., 23.), tile.max - Vec2::splat(5.));
        if viewport.height() < 4. {
            continue;
        }
        if let Some(scene) = scenes.iter().find(|s| s.name == w.name) {
            let scale = (viewport.width() / scene.rect.width.max(1.))
                .min(viewport.height() / scene.rect.height.max(1.));
            let origin =
                viewport.center() - Vec2::new(scene.rect.width, scene.rect.height) * scale / 2.;
            let clip = clip.with_clip_rect(viewport);
            for window in &scene.windows {
                let r = Rect::from_min_size(
                    origin
                        + Vec2::new(window.rect.x - scene.rect.x, window.rect.y - scene.rect.y)
                            * scale,
                    Vec2::new(window.rect.width, window.rect.height) * scale,
                )
                .shrink(1.);
                if !r.is_positive() {
                    continue;
                }
                clip.rect_filled(
                    r,
                    2.,
                    if window.floating {
                        Color32::from_rgb(57, 48, 66)
                    } else {
                        Color32::from_rgb(40, 53, 63)
                    },
                );
                clip.rect_stroke(
                    r,
                    2.,
                    Stroke::new(1_f32, if window.focused { color } else { LINE }),
                    StrokeKind::Inside,
                );
                if r.width() > 45. && r.height() > 16. {
                    clip.with_clip_rect(r.shrink(3.)).text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        &window.title,
                        FontId::proportional(9.),
                        TEXT,
                    );
                }
            }
        }
    }
}
fn demo_scenes() -> Vec<live::Workspace> {
    let geometry = |x, y, width, height| live::Geometry {
        x,
        y,
        width,
        height,
    };
    [
        (
            "1: code",
            "DP-1",
            vec![
                ("Editor", 0., 0., 1700., 1440.),
                ("Terminal", 1700., 0., 860., 1440.),
            ],
        ),
        ("2: web", "DP-1", vec![("Browser", 0., 0., 2560., 1440.)]),
        ("3: chat", "eDP-1", vec![("Chat", 0., 0., 2560., 1440.)]),
    ]
    .into_iter()
    .map(|(name, output, windows)| live::Workspace {
        name: name.into(),
        output: output.into(),
        rect: geometry(0., 0., 2560., 1440.),
        windows: windows
            .into_iter()
            .enumerate()
            .map(|(i, (title, x, y, w, h))| live::Window {
                title: title.into(),
                rect: geometry(x, y, w, h),
                focused: i == 0,
                floating: false,
            })
            .collect(),
    })
    .collect()
}
