use crate::{
    capture,
    icons::{self, Icon},
    integration, ipc,
    model::{Layout, Mode, Output, Rect as OutputRect, Workspace, snap},
    preview::Preview,
    store,
    theme::*,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

const PREVIEW_FILTER: egui::TextureOptions =
    egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));

const COLORS: [Color32; 4] = [ACCENT, TEAL, LAVENDER, WARN];

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
    pan_drag: Option<(Pos2, Vec2)>,
    drag: Option<Drag>,
    help: bool,
    new_workspace: String,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    receiver: Option<std::sync::mpsc::Receiver<anyhow::Result<Vec<capture::Screen>>>>,
    refresh_receiver: Option<std::sync::mpsc::Receiver<anyhow::Result<Layout>>>,
    live_error: Option<String>,
    config_patch: Option<integration::Patch>,
    config_message: String,
}
impl App {
    pub fn new(ctx: &egui::Context, demo: bool) -> anyhow::Result<Self> {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Inter".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")).into(),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "Inter".into());
        ctx.set_fonts(fonts);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = SELECTED;
        visuals.selection.stroke = Stroke::new(1_f32, ACCENT);
        visuals.widgets.inactive.bg_fill = PANEL;
        visuals.widgets.inactive.weak_bg_fill = PANEL;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1_f32, LINE);
        visuals.widgets.hovered.bg_fill = LINE;
        visuals.widgets.hovered.weak_bg_fill = LINE;
        visuals.widgets.active.bg_fill = SELECTED;
        visuals.widgets.active.weak_bg_fill = SELECTED;
        ctx.set_visuals(visuals);
        ctx.tessellation_options_mut(|options| {
            options.feathering = true;
            options.feathering_size_in_pixels = 1.;
        });
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
            pan_drag: None,
            drag: None,
            help: false,
            new_workspace: String::new(),
            textures: std::collections::HashMap::new(),
            refresh_receiver: None,
            receiver: if demo {
                None
            } else {
                Some(capture::poll(ctx.clone()))
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
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.refresh_receiver.is_some() {
            return;
        }
        let (send, receive) = std::sync::mpsc::channel();
        self.refresh_receiver = Some(receive);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(ipc::snapshot());
            ctx.request_repaint();
        });
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
        self.pan_drag = None;
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
                self.config_message.clear();
                if patch.installed() {
                    self.help = false;
                } else {
                    self.config_patch = Some(patch);
                }
            }
            Err(e) => self.config_message = format!("{e:#}"),
        }
    }
    fn read_live(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.receiver {
            while let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(screens) => {
                        let names: std::collections::HashSet<_> =
                            screens.iter().map(|s| s.name.clone()).collect();
                        self.textures.retain(|name, _| names.contains(name));
                        for screen in screens {
                            if let Some(texture) = self.textures.get_mut(&screen.name) {
                                texture.set(screen.image, PREVIEW_FILTER);
                            } else {
                                self.textures.insert(
                                    screen.name.clone(),
                                    ctx.load_texture(screen.name, screen.image, PREVIEW_FILTER),
                                );
                            }
                        }
                        self.live_error = None;
                    }
                    Err(e) => {
                        self.textures.clear();
                        self.live_error = Some(format!("Screen previews: {e:#}"));
                    }
                }
            }
        }
        if let Some(receiver) = &self.refresh_receiver
            && let Ok(result) = receiver.try_recv()
        {
            match result {
                Ok(layout) => {
                    let selected = self
                        .layout
                        .outputs
                        .get(self.selected)
                        .map(|o| o.name.clone());
                    self.selected = layout
                        .outputs
                        .iter()
                        .position(|o| Some(&o.name) == selected.as_ref())
                        .unwrap_or(0);
                    self.layout = layout.clone();
                    self.baseline = layout;
                    self.drag = None;
                    self.fit();
                    self.status.clear();
                }
                Err(e) => self.status = format!("Refresh failed: {e:#}"),
            }
            self.refresh_receiver = None;
        }
    }
    pub fn draw(&mut self, ctx: &egui::Context) {
        self.read_live(ctx);
        if self.preview.as_ref().is_some_and(|p| p.remaining() == 0) {
            self.finish(false);
        }
        if self.preview.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::new().fill(BG).inner_margin(16.))
            .show(ctx, |ui| {
                let (logo, _) = ui.allocate_exact_size(Vec2::new(34., 30.), Sense::hover());
                let screen =
                    Rect::from_min_size(logo.left_top() + Vec2::new(0., 2.), Vec2::new(30., 20.));
                ui.painter().rect_stroke(
                    screen,
                    3.,
                    Stroke::new(1.5_f32, ACCENT),
                    StrokeKind::Inside,
                );
                ui.painter().line_segment(
                    [
                        logo.left_top() + Vec2::new(7., 27.),
                        logo.left_top() + Vec2::new(23., 27.),
                    ],
                    Stroke::new(1.5_f32, ACCENT),
                );
                ui.add_space(6.);
                ui.add_enabled_ui(self.preview.is_none(), |ui| {
                    ui.horizontal(|ui| {
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
                        if icons::enabled_button(
                            ui,
                            Icon::Refresh,
                            "Refresh displays from Sway",
                            self.refresh_receiver.is_some(),
                            !self.demo && self.refresh_receiver.is_none(),
                        )
                        .clicked()
                        {
                            self.refresh(ctx);
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(16.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(p) = &self.preview {
                        ui.label(
                            RichText::new(format!("{}s", p.remaining()))
                                .monospace()
                                .color(ACCENT),
                        );
                    } else if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).size(12.).color(WARN));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 12.;
                        if self.preview.is_some() {
                            if icons::button(ui, Icon::Confirm, "Confirm & save", true).clicked() {
                                self.finish(true);
                            }
                            if icons::button(ui, Icon::Revert, "Revert preview", false).clicked() {
                                self.finish(false);
                            }
                        } else {
                            if icons::enabled_button(
                                ui,
                                Icon::Save,
                                "Save layout · confirm within 20 seconds",
                                true,
                                !self.demo && self.refresh_receiver.is_none(),
                            )
                            .clicked()
                            {
                                self.preview();
                            }
                            if icons::button(ui, Icon::Reset, "Reset edits", false).clicked() {
                                self.layout = self.baseline.clone();
                                self.selected = 0;
                                self.drag = None;
                                self.fit();
                                self.status.clear();
                            }
                            if icons::enabled_button(
                                ui,
                                Icon::Load,
                                "Load saved layout",
                                false,
                                !self.demo,
                            )
                            .clicked()
                            {
                                self.load_saved();
                            }
                        }
                    });
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
                    ui.spacing_mut().item_spacing.y = 7.;
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
                        ui.label(RichText::new(warning).size(12.).color(WARN));
                    }
                    if let Some(error) = &self.live_error {
                        ui.label(RichText::new(error).size(12.).color(WARN));
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
        let mut close = false;
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
                    if !self.help {
                        close = true;
                    }
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
                                            '+' => ACCENT,
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
                                Ok(()) => {
                                    self.check_config();
                                    close = true;
                                }
                                Err(e) => self.config_message = format!("{e:#}"),
                            }
                        }
                    }
                }
                if !self.config_message.is_empty() {
                    ui.label(&self.config_message);
                }
            });
        self.help = open && !close;
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("DISPLAYS").size(11.).color(MUTED));
        for (i, o) in self.layout.outputs.iter_mut().enumerate() {
            egui::Frame::new()
                .fill(if self.selected == i { SELECTED } else { PANEL })
                .corner_radius(6.)
                .inner_margin(8.)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 46.).max(40.);
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::new(width, 26.), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                &o.name,
                            )
                        });
                        ui.painter().text(
                            rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            &o.name,
                            egui::FontId::proportional(14.),
                            if self.selected == i { ACCENT } else { TEXT },
                        );
                        if response.clicked() {
                            self.selected = i;
                        }
                        icons::switch(ui, &mut o.active, &format!("Enable {}", o.name));
                    });
                });
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
        ui.add_space(12.);
        ui.label(RichText::new("Resolution & refresh").size(12.).color(MUTED));
        ui.scope(|ui| {
            ui.style_mut().spacing.button_padding = Vec2::new(12., 10.);
            ui.style_mut().visuals.widgets.inactive.bg_fill = BG;
            ui.style_mut().visuals.widgets.inactive.weak_bg_fill = BG;
            ui.style_mut().visuals.widgets.inactive.bg_stroke = Stroke::new(1_f32, LINE);
            ui.style_mut().visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
            egui::ComboBox::from_id_salt(("mode", &o.name))
                .width(ui.available_width())
                .selected_text(
                    RichText::new(
                        o.current_mode
                            .as_ref()
                            .map(Mode::label)
                            .unwrap_or_else(|| "Select mode".into()),
                    )
                    .size(12.),
                )
                .show_ui(ui, |ui| {
                    for mode in &o.modes {
                        ui.selectable_value(&mut o.current_mode, Some(mode.clone()), mode.label());
                    }
                });
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
        ui.add_space(12.);
        let mut mirrored = flipped;
        ui.label(RichText::new("Mirror").size(12.).color(MUTED));
        if icons::switch(ui, &mut mirrored, "Mirror display").changed() {
            o.transform = transform(angle, mirrored);
        }
        if let Err(e) = self.layout.validate() {
            ui.add_space(12.);
            ui.label(RichText::new(e.to_string()).color(WARN));
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
        if ui.is_enabled()
            && ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)))
            && self.drag.is_none()
            && self.pan_drag.is_none()
        {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.01 {
                let factor = (scroll * 0.003).exp();
                let next = (self.zoom * factor).clamp(0.3, 4.);
                let ratio = next / self.zoom;
                let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                self.pan = (pointer - rect.center()) * (1. - ratio) + self.pan * ratio;
                self.zoom = next;
                ui.ctx().request_repaint();
            }
        }
        let screen_rects: Vec<_> = active
            .iter()
            .map(|(_, o)| {
                let (w, h) = o.size();
                Rect::from_min_size(
                    origin + Vec2::new(o.rect.x as f32, o.rect.y as f32) * scale,
                    Vec2::new(w as f32, h as f32) * scale,
                )
            })
            .collect();
        if ui.is_enabled() && self.drag.is_none() {
            let start = ui.input(|i| {
                let origin = i.pointer.press_origin()?;
                let primary = i.pointer.primary_pressed();
                let pan = i.pointer.button_pressed(egui::PointerButton::Middle)
                    || (primary
                        && (i.key_down(egui::Key::Space)
                            || !screen_rects.iter().any(|r| r.contains(origin))));
                (pan && rect.contains(origin)).then_some(origin)
            });
            if let Some(origin) = start {
                self.pan_drag = Some((origin, self.pan));
            }
        }
        let pan_gesture = self.pan_drag.is_some();
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
                if pan_gesture {
                    Sense::hover()
                } else {
                    Sense::click_and_drag()
                },
            );
            if interaction.clicked() {
                self.selected = i;
            }
            if interaction.drag_started_by(egui::PointerButton::Primary) {
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
            // Keep the border inside its own display so touching monitors never cover each other.
            painter.rect_filled(r, 0., PANEL);
            if let Some(texture) = self.textures.get(&o.name) {
                painter.image(
                    texture.id(),
                    r,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                    Color32::WHITE,
                );
            } else {
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    if self.demo {
                        "Demo display"
                    } else {
                        "Waiting for screen…"
                    },
                    FontId::proportional(12.),
                    MUTED,
                );
            }
            painter.rect_stroke(
                r,
                0.,
                Stroke::new(
                    if selected { 2_f32 } else { 1_f32 },
                    if selected { color } else { LINE },
                ),
                StrokeKind::Inside,
            );
            if r.width() > 100. {
                let label = format!("{}   {w} × {h}", o.name);
                let galley = painter.layout_no_wrap(label, FontId::proportional(12.), color);
                let size = galley.size();
                let positions = [
                    r.left_top() - Vec2::new(0., size.y + 10.),
                    r.left_bottom() + Vec2::new(0., 10.),
                    r.left_center() - Vec2::new(size.x + 10., size.y / 2.),
                    r.right_center() + Vec2::new(10., -size.y / 2.),
                ];
                for position in positions {
                    let badge = Rect::from_min_size(position, size).expand2(Vec2::new(6., 3.));
                    if rect.contains(badge.min)
                        && rect.contains(badge.max)
                        && !screen_rects.iter().any(|screen| screen.intersects(badge))
                    {
                        painter.rect_filled(badge, 4., PANEL);
                        painter.galley(position, galley, color);
                        break;
                    }
                }
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
        } else if let Some((start, offset)) = self.pan_drag {
            if let Some(position) = ui.input(|i| i.pointer.interact_pos()) {
                self.pan = offset + (position - start);
                ui.ctx().request_repaint();
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if !ui.input(|i| i.pointer.primary_down() || i.pointer.middle_down()) {
                self.pan_drag = None;
            }
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grabbing_background_or_middle_dragging_a_screen_pans_without_moving_displays() {
        for (button, start) in [
            (egui::PointerButton::Primary, Pos2::new(250., 180.)),
            (egui::PointerButton::Middle, Pos2::new(450., 400.)),
        ] {
            let ctx = egui::Context::default();
            let mut app = App::new(&ctx, true).unwrap();
            let original = serde_json::to_string(&app.layout).unwrap();
            let mut draw = |events| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.draw(ctx),
                );
            };
            draw(vec![]);
            draw(vec![egui::Event::PointerMoved(start)]);
            draw(vec![egui::Event::PointerButton {
                pos: start,
                button,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            let end = start + Vec2::new(70., 40.);
            draw(vec![egui::Event::PointerMoved(end)]);
            draw(vec![egui::Event::PointerButton {
                pos: end,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert_eq!(app.pan, Vec2::new(70., 40.));
            assert_eq!(serde_json::to_string(&app.layout).unwrap(), original);
            assert!(app.drag.is_none());
        }
    }
    #[test]
    fn scrolling_over_display_pixels_zooms_without_editing_the_layout() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, true).unwrap();
        let original = serde_json::to_string(&app.layout).unwrap();
        let input = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input(vec![]), |ctx| app.draw(ctx));
        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(Pos2::new(450., 400.))]),
            |ctx| app.draw(ctx),
        );
        let _ = ctx.run(
            input(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., 100.),
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| app.draw(ctx),
        );
        assert!(
            app.zoom > 1.,
            "Wheel events over a display must reach the canvas"
        );
        assert_eq!(serde_json::to_string(&app.layout).unwrap(), original);
    }
}
