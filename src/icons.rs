use crate::theme::*;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Row,
    Fit,
    Snap,
    Minus,
    Plus,
    Zoom,
    Refresh,
    Load,
    Reset,
    Preview,
    Save,
    Revert,
    Rotation(u16),
    Flip,
}

pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
    enabled_button(ui, icon, label, active, true)
}
pub fn enabled_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(Vec2::splat(36.))
            .fill(if active { SELECTED } else { PANEL })
            .stroke(Stroke::new(1_f32, if active { ACCENT } else { LINE }))
            .corner_radius(6.),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let color = if !enabled {
        LINE
    } else if active {
        ACCENT
    } else {
        TEXT
    };
    paint(ui.painter(), response.rect.shrink(7.), icon, color);
    response.on_hover_text(label)
}
/// Compact, keyboard-accessible switch with a predictable hit target.
pub fn switch(ui: &mut egui::Ui, checked: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(36., 26.), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, ui.is_enabled(), label)
    });
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let track = rect.shrink2(Vec2::new(0., 3.));
    let enabled = ui.is_enabled();
    ui.painter()
        .rect_filled(track, 10., if *checked && enabled { SELECTED } else { BG });
    ui.painter().rect_stroke(
        track,
        10.,
        Stroke::new(1_f32, if *checked && enabled { ACCENT } else { LINE }),
        StrokeKind::Inside,
    );
    let x = if *checked {
        track.right() - 10.
    } else {
        track.left() + 10.
    };
    ui.painter().circle_filled(
        Pos2::new(x, track.center().y),
        6.,
        if !enabled {
            LINE
        } else if *checked {
            ACCENT
        } else {
            MUTED
        },
    );
    response.on_hover_text(label)
}

pub fn paint(p: &egui::Painter, r: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.6_f32, color);
    let point = |x: f32, y: f32| Pos2::new(r.left() + r.width() * x, r.top() + r.height() * y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([point(a.0, a.1), point(b.0, b.1)], s);
    };
    let screen = |a: (f32, f32), b: (f32, f32)| {
        p.rect_stroke(
            Rect::from_min_max(point(a.0, a.1), point(b.0, b.1)),
            2.,
            s,
            StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Row => {
            screen((0., 0.38), (0.38, 0.82));
            screen((0.46, 0.14), (1., 0.82));
            line((0., 0.02), (1., 0.02));
            line((0., 0.98), (1., 0.98));
        }
        Icon::Fit => {
            for (x, y, dx, dy) in [
                (0.1, 0.1, 0.25, 0.25),
                (0.9, 0.1, -0.25, 0.25),
                (0.1, 0.9, 0.25, -0.25),
                (0.9, 0.9, -0.25, -0.25),
            ] {
                line((x, y), (x + dx, y));
                line((x, y), (x, y + dy));
            }
        }
        Icon::Snap => {
            line((0.5, 0.2), (0.5, 0.8));
            line((0., 0.5), (0.3, 0.5));
            line((0.7, 0.5), (1., 0.5));
            for (a, b) in [
                ((0.34, 0.2), (0.22, 0.04)),
                ((0.66, 0.2), (0.78, 0.04)),
                ((0.34, 0.8), (0.22, 0.96)),
                ((0.66, 0.8), (0.78, 0.96)),
            ] {
                line(a, b);
            }
        }
        Icon::Minus | Icon::Plus => {
            line((0.2, 0.5), (0.8, 0.5));
            if matches!(icon, Icon::Plus) {
                line((0.5, 0.2), (0.5, 0.8));
            }
        }
        Icon::Zoom => {
            p.circle_stroke(point(0.4, 0.4), r.width() * 0.28, s);
            line((0.62, 0.62), (0.95, 0.95));
        }
        Icon::Refresh | Icon::Reset => {
            let pts = (0..=18)
                .map(|i| {
                    let a = -0.6 + i as f32 / 18. * 5.2;
                    point(0.5 + a.cos() * 0.34, 0.5 + a.sin() * 0.34)
                })
                .collect();
            p.add(egui::Shape::line(pts, s));
            line((0.8, 0.09), (0.8, 0.37));
            line((0.8, 0.37), (0.53, 0.37));
            if matches!(icon, Icon::Reset) {
                line((0.35, 0.5), (0.65, 0.5));
            }
        }
        Icon::Load => {
            screen((0.08, 0.4), (0.92, 0.92));
            line((0.5, 0.03), (0.5, 0.63));
            line((0.3, 0.43), (0.5, 0.63));
            line((0.5, 0.63), (0.7, 0.43));
        }
        Icon::Preview => {
            screen((0., 0.12), (1., 0.82));
            p.add(egui::Shape::closed_line(
                vec![point(0.4, 0.3), point(0.4, 0.64), point(0.67, 0.47)],
                s,
            ));
            line((0.5, 0.82), (0.5, 0.98));
        }
        Icon::Save => {
            screen((0.08, 0.08), (0.92, 0.92));
            screen((0.28, 0.08), (0.72, 0.38));
            screen((0.25, 0.62), (0.75, 0.92));
        }
        Icon::Revert => {
            line((0.2, 0.2), (0.8, 0.8));
            line((0.8, 0.2), (0.2, 0.8));
        }
        Icon::Rotation(angle) => {
            let rotate = |x: f32, y: f32| {
                let a = (angle as f32).to_radians();
                let x = x - 0.5;
                let y = y - 0.5;
                point(
                    0.5 + x * a.cos() - y * a.sin(),
                    0.5 + x * a.sin() + y * a.cos(),
                )
            };
            p.add(egui::Shape::closed_line(
                vec![
                    rotate(0.1, 0.25),
                    rotate(0.9, 0.25),
                    rotate(0.9, 0.75),
                    rotate(0.1, 0.75),
                ],
                s,
            ));
            p.line_segment(
                [rotate(0.24, 0.59), rotate(0.38, 0.59)],
                Stroke::new(2.4_f32, color),
            );
        }
        Icon::Flip => {
            line((0.5, 0.05), (0.5, 0.95));
            p.add(egui::Shape::closed_line(
                vec![point(0.05, 0.8), point(0.35, 0.8), point(0.35, 0.2)],
                s,
            ));
            p.add(egui::Shape::closed_line(
                vec![point(0.65, 0.2), point(0.65, 0.8), point(0.95, 0.8)],
                s,
            ));
        }
    }
}
pub fn scale(ui: &mut egui::Ui, value: &mut f64) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Scale").color(MUTED).size(12.));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{value:.2}×"))
                    .monospace()
                    .color(ACCENT),
            );
        });
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        if button(ui, Icon::Minus, "Decrease scale by 0.25", false).clicked() {
            *value = (*value - 0.25).max(0.5);
        }
        let (r, response) = ui.allocate_exact_size(Vec2::new(164., 36.), Sense::click_and_drag());
        let rail = r.shrink2(Vec2::new(8., 0.));
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let fraction = ((pos.x - rail.left()) / rail.width()).clamp(0., 1.) as f64;
            *value = (0.5 + (fraction * 14.).round() * 0.25).clamp(0.5, 4.);
            response.request_focus();
        }
        if response.has_focus() {
            let step = ui.input(|i| {
                i.key_pressed(egui::Key::ArrowRight) as i32
                    - i.key_pressed(egui::Key::ArrowLeft) as i32
            });
            *value = (*value + step as f64 * 0.25).clamp(0.5, 4.);
        }
        let p = ui.painter();
        let y = r.center().y;
        p.line_segment(
            [Pos2::new(rail.left(), y), Pos2::new(rail.right(), y)],
            Stroke::new(4_f32, LINE),
        );
        for i in 0..=14 {
            let x = rail.left() + rail.width() * i as f32 / 14.;
            p.line_segment(
                [Pos2::new(x, y + 7.), Pos2::new(x, y + 10.)],
                Stroke::new(1_f32, MUTED),
            );
        }
        let x = rail.left() + rail.width() * ((*value - 0.5) / 3.5) as f32;
        p.line_segment(
            [Pos2::new(rail.left(), y), Pos2::new(x, y)],
            Stroke::new(4_f32, ACCENT),
        );
        p.circle_filled(Pos2::new(x, y), 7., ACCENT);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), "Display scale")
        });
        response.on_hover_text("Scale · 0.25× steps");
        if button(ui, Icon::Plus, "Increase scale by 0.25", false).clicked() {
            *value = (*value + 0.25).min(4.);
        }
    });
}
