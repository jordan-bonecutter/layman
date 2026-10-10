use crate::{
    ipc,
    model::{Layout, Mode, Output, Rect as OutputRect, TRANSFORMS, Workspace, snap},
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
            status: if demo {
                "Demo · explore freely; Sway and saved settings are untouched.".into()
            } else {
                "Connected to Sway. Your current layout is ready to edit.".into()
            },
            preview: None,
            snapping: true,
            zoom: 1.,
            pan: Vec2::ZERO,
            drag: None,
            help: false,
            new_workspace: String::new(),
        })
    }
    fn message(&mut self, result: anyhow::Result<()>, success: &str) {
        self.status = match result {
            Ok(()) => success.into(),
            Err(e) => format!("{e:#}"),
        };
    }
    fn dirty(&self) -> bool {
        serde_json::to_string(&self.layout).ok() != serde_json::to_string(&self.baseline).ok()
    }
    fn refresh(&mut self) {
        match ipc::snapshot() {
            Ok(l) => {
                self.layout = l.clone();
                self.baseline = l;
                self.selected = 0;
                self.fit();
                self.status = "Read the current layout from Sway.".into();
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
        self.message(
            result,
            "Saved layout loaded into the editor. Preview to apply it.",
        );
    }
    fn fit(&mut self) {
        self.zoom = 1.;
        self.pan = Vec2::ZERO;
    }
    fn preview(&mut self) {
        match Preview::start(&self.layout) {
            Ok(p) => {
                self.preview = Some(p);
                self.status =
                    "Preview is live. Keep it within 20 seconds, or it will revert.".into();
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
            self.message(
                result,
                if keep {
                    "Layout applied and saved. Add the include once to survive Sway reloads."
                } else {
                    "Previous layout restored. Your draft is still here."
                },
            );
        }
    }
    pub fn draw(&mut self, ctx: &egui::Context) {
        if self.preview.as_ref().is_some_and(|p| p.remaining() == 0) {
            self.finish(false);
        }
        if self.preview.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(BG).inner_margin(22.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (logo, _) = ui.allocate_exact_size(Vec2::new(26., 28.), Sense::hover());
                    let screen = Rect::from_min_size(
                        logo.left_top() + Vec2::new(0., 4.),
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
                    ui.vertical(|ui| {
                        ui.label(RichText::new("layman").size(25.).strong());
                        ui.label(
                            RichText::new("A little order for your desktop.")
                                .size(12.)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reload-proof setup").clicked() {
                            self.help = true;
                        }
                        ui.label(
                            RichText::new(if self.demo { "DEMO" } else { "SWAY CONNECTED" })
                                .size(11.)
                                .color(MINT),
                        );
                    });
                });
            });
        egui::TopBottomPanel::bottom("footer").frame(egui::Frame::new().fill(BG).inner_margin(18.)).show(ctx, |ui| {
            if let Some(p) = &self.preview {
                let remaining = p.remaining();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Keep this layout?  {remaining}s")).strong().color(MINT));
                    if ui.button("Revert now").clicked() { self.finish(false); }
                    if ui.add(egui::Button::new(RichText::new("Keep & save").color(BG).strong()).fill(MINT)).clicked() { self.finish(true); }
                });
                ui.label(RichText::new("The separate rollback helper restores your displays even if this window closes.").size(12.).color(MUTED));
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if self.dirty() { "Unsaved changes" } else { "In sync" }).color(if self.dirty() { COLORS[2] } else { MINT }).size(12.));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!self.demo, egui::Button::new(RichText::new("Preview layout").color(BG).strong()).fill(MINT)).clicked() { self.preview(); }
                        if ui.button("Reset edits").clicked() { self.layout = self.baseline.clone(); self.selected = 0; self.fit(); }
                        if ui.add_enabled(!self.demo, egui::Button::new("Load saved")).clicked() { self.load_saved(); }
                    });
                });
                ui.label(RichText::new(&self.status).size(12.).color(MUTED));
            }
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
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Your space, arranged.").size(24.).strong());
                            ui.label(
                                RichText::new(
                                    "Drag displays to match your desk. Edges snap together.",
                                )
                                .color(MUTED)
                                .size(13.),
                            );
                        });
                    });
                    ui.add_space(10.);
                    ui.horizontal(|ui| {
                        if ui.button("Arrange in a row").clicked() {
                            self.layout.horizontal();
                            self.fit();
                        }
                        if ui.button("Fit").clicked() {
                            self.fit();
                        }
                        ui.checkbox(&mut self.snapping, "Snap");
                        if ui.small_button("−").clicked() {
                            self.zoom = (self.zoom / 1.2).max(0.3);
                        }
                        ui.label(
                            RichText::new(format!("{:.0}%", self.zoom * 100.))
                                .color(MUTED)
                                .size(12.),
                        );
                        if ui.small_button("+").clicked() {
                            self.zoom = (self.zoom * 1.2).min(4.);
                        }
                        if ui
                            .add_enabled(!self.demo, egui::Button::new("Refresh Sway"))
                            .clicked()
                        {
                            self.refresh();
                        }
                    });
                    ui.add_space(10.);
                    let warnings = self.layout.warnings();
                    for warning in warnings {
                        ui.label(RichText::new(warning).size(12.).color(COLORS[2]));
                    }
                    ui.add_space(8.);
                    self.canvas(ui);
                });
            });
        if self.help {
            egui::Window::new("Make it stick").collapsible(false).resizable(false).default_width(530.).open(&mut self.help).show(ctx, |ui| {
                ui.label(RichText::new("Sway already has everything we need.").size(21.).strong());
                ui.add_space(8.);
                ui.label("1. Preview a layout, then choose Keep & save.\n2. Add this line at the very end of your Sway config:\n3. Reload Sway. Your layout will return on every reload.");
                let line = store::include_line().unwrap_or_else(|e| e.to_string());
                ui.add_space(8.);
                ui.code(&line);
                if ui.button("Copy include line").clicked() { ui.ctx().copy_text(line); }
                ui.add_space(8.);
                ui.label(RichText::new("No systemd service. No background daemon.").color(MINT));
                ui.label("The generated include restores display settings. Its exec_always command moves existing workspaces, carrying their tiled and floating windows with them. Tiling trees stay intact.");
                ui.label(RichText::new("Keep the installed binary at the same path. Save again after moving it. Place this include after other display/workspace settings.").size(12.).color(MUTED));
            });
        }
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
        ui.label(RichText::new("Scale").size(12.).color(MUTED));
        ui.add(
            egui::Slider::new(&mut o.scale, 0.5..=4.)
                .step_by(0.05)
                .suffix("×"),
        );
        ui.add_space(8.);
        ui.label(RichText::new("Rotation / flip").size(12.).color(MUTED));
        egui::ComboBox::from_id_salt("rotation")
            .width(235.)
            .selected_text(&o.transform)
            .show_ui(ui, |ui| {
                for t in TRANSFORMS {
                    ui.selectable_value(&mut o.transform, t.to_string(), t);
                }
            });
        ui.add_space(8.);
        ui.label(
            RichText::new("Position · logical pixels")
                .size(12.)
                .color(MUTED),
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut o.rect.x)
                    .speed(10.)
                    .range(-100000..=100000)
                    .prefix("X  "),
            );
            ui.add(
                egui::DragValue::new(&mut o.rect.y)
                    .speed(10.)
                    .range(-100000..=100000)
                    .prefix("Y  "),
            );
        });
        let (w, h) = o.size();
        ui.label(
            RichText::new(format!("Desktop area: {w} × {h}"))
                .size(12.)
                .color(MUTED),
        );
        ui.add_space(12.);
        ui.label(RichText::new("Tip: hold Alt while dragging to skip snapping. Use the arrow keys to nudge the selected display (Shift = 10 px).").size(12.).color(MUTED));
        if let Err(e) = self.layout.validate() {
            ui.add_space(12.);
            ui.label(RichText::new(e.to_string()).color(COLORS[2]));
        }
    }
    fn workspaces(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("PUT YOUR APPS IN PLACE")
                .size(11.)
                .color(MUTED),
        );
        ui.label("Assign a workspace to a display. Its windows move together when you apply.");
        ui.add_space(8.);
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
        ui.add_space(12.);
        ui.label(RichText::new("Future workspaces are assigned by the saved Sway include. Apply only moves workspaces that already exist.").size(12.).color(MUTED));
    }
    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(Vec2::new(200., 240.)),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 16., Color32::from_rgb(20, 25, 32));
        painter.rect_stroke(rect, 16., Stroke::new(1_f32, LINE), StrokeKind::Inside);
        let spacing = 24.;
        let mut x = rect.left() + 12.;
        while x < rect.right() {
            let mut y = rect.top() + 12.;
            while y < rect.bottom() {
                painter.circle_filled(Pos2::new(x, y), 1., LINE);
                y += spacing;
            }
            x += spacing;
        }
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
            if r.width() > 100. && r.height() > 80. {
                painter.text(
                    inner.left_top(),
                    Align2::LEFT_TOP,
                    format!("{:02}", i + 1),
                    FontId::proportional(15.),
                    color,
                );
                painter.text(
                    r.center() - Vec2::new(0., 13.),
                    Align2::CENTER_CENTER,
                    &o.name,
                    FontId::proportional(19.),
                    TEXT,
                );
                painter.text(
                    r.center() + Vec2::new(0., 12.),
                    Align2::CENTER_CENTER,
                    format!("{w} × {h}  /  {}×", o.scale),
                    FontId::proportional(12.),
                    MUTED,
                );
                let workspaces = self
                    .layout
                    .workspaces
                    .iter()
                    .filter(|w| w.output == o.name)
                    .map(|w| w.name.as_str())
                    .collect::<Vec<_>>()
                    .join("   ");
                painter.text(
                    inner.left_bottom(),
                    Align2::LEFT_BOTTOM,
                    format!("workspaces  {workspaces}"),
                    FontId::proportional(11.),
                    color,
                );
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
        painter.text(
            rect.left_bottom() + Vec2::new(16., -14.),
            Align2::LEFT_BOTTOM,
            "LOGICAL PIXELS  ·  ALT = FREE DRAG  ·  MIDDLE DRAG = PAN",
            FontId::proportional(10.),
            MUTED,
        );
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
