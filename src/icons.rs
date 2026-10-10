use crate::theme::*;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Row,
    Fit,
    Snap,
    Refresh,
    Load,
    Reset,
    Confirm,
    Save,
    Revert,
    Rotation(u16),
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

/// A consistent 24-unit vector set with rounded strokes and generous negative space.
pub fn paint(p: &egui::Painter, r: Rect, icon: Icon, color: Color32) {
    let unit = r.width().min(r.height()) / 24.;
    let origin = r.center() - Vec2::splat(unit * 12.);
    let point = |x: f32, y: f32| origin + Vec2::new(x, y) * unit;
    let stroke = Stroke::new(1.8 * unit, color);
    let path = |coords: &[(f32, f32)]| {
        let points: Vec<_> = coords.iter().map(|&(x, y)| point(x, y)).collect();
        if let (Some(&first), Some(&last)) = (points.first(), points.last()) {
            p.add(egui::Shape::line(points, stroke));
            p.circle_filled(first, stroke.width / 2., color);
            p.circle_filled(last, stroke.width / 2., color);
        }
    };
    let outline = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_stroke(
            Rect::from_min_size(point(x, y), Vec2::new(w, h) * unit),
            2.,
            stroke,
            StrokeKind::Middle,
        );
    };
    let arc = |cx: f32, cy: f32, radius: f32, start: f32, end: f32| {
        let coords: Vec<_> = (0..=32)
            .map(|i| {
                let a = (start + (end - start) * i as f32 / 32.).to_radians();
                (cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect();
        path(&coords);
    };
    match icon {
        Icon::Row => {
            outline(2., 7., 8., 10.);
            outline(14., 4., 8., 13.);
            path(&[(6., 17.), (6., 20.)]);
            path(&[(18., 17.), (18., 20.)]);
            path(&[(3., 20.), (9., 20.)]);
            path(&[(15., 20.), (21., 20.)]);
        }
        Icon::Fit => {
            path(&[(8., 3.), (3., 3.), (3., 8.)]);
            path(&[(16., 3.), (21., 3.), (21., 8.)]);
            path(&[(3., 16.), (3., 21.), (8., 21.)]);
            path(&[(16., 21.), (21., 21.), (21., 16.)]);
        }
        Icon::Snap => {
            // A magnet: recognizable even when the control is inactive.
            path(&[(5., 10.), (5., 4.), (9., 4.), (9., 10.)]);
            path(&[(15., 10.), (15., 4.), (19., 4.), (19., 10.)]);
            arc(12., 13., 7., 0., 180.);
            arc(12., 13., 3., 0., 180.);
            path(&[(5., 10.), (5., 13.)]);
            path(&[(9., 10.), (9., 13.)]);
            path(&[(15., 10.), (15., 13.)]);
            path(&[(19., 10.), (19., 13.)]);
            path(&[(5., 9.), (9., 9.)]);
            path(&[(15., 9.), (19., 9.)]);
        }
        Icon::Refresh => {
            arc(12., 12., 8., -150., -20.);
            path(&[(19.5, 3.), (19.5, 9.3), (13.5, 9.3)]);
            arc(12., 12., 8., 30., 160.);
            path(&[(4.5, 21.), (4.5, 14.7), (10.5, 14.7)]);
        }
        Icon::Reset => {
            path(&[(8., 3.), (3., 8.), (8., 13.)]);
            path(&[(3., 8.), (14., 8.)]);
            arc(14., 14., 6., -90., 90.);
            path(&[(14., 20.), (9., 20.)]);
        }
        Icon::Load => {
            path(&[
                (3., 10.),
                (3., 5.),
                (9., 5.),
                (12., 8.),
                (21., 8.),
                (21., 10.),
            ]);
            path(&[(3., 10.), (21., 10.), (19., 20.), (5., 20.), (3., 10.)]);
        }
        Icon::Confirm => path(&[(4., 12.), (9., 17.), (20., 6.)]),
        Icon::Save => {
            path(&[
                (5., 3.),
                (17., 3.),
                (21., 7.),
                (21., 21.),
                (3., 21.),
                (3., 3.),
                (5., 3.),
            ]);
            path(&[(7., 3.), (7., 9.), (16., 9.), (16., 3.)]);
            outline(7., 14., 10., 7.);
            path(&[(13., 5.), (13., 7.)]);
        }
        Icon::Revert => {
            path(&[(6., 6.), (18., 18.)]);
            path(&[(18., 6.), (6., 18.)]);
        }
        Icon::Rotation(angle) => {
            let rotate = |x: f32, y: f32| {
                let a = (angle as f32).to_radians();
                let (x, y) = (x - 12., y - 12.);
                (
                    12. + x * a.cos() - y * a.sin(),
                    12. + x * a.sin() + y * a.cos(),
                )
            };
            let frame: Vec<_> = [(3., 6.), (21., 6.), (21., 18.), (3., 18.), (3., 6.)]
                .into_iter()
                .map(|(x, y)| rotate(x, y))
                .collect();
            path(&frame);
            // The arrow identifies the top of the panel in all four orientations.
            let arrow: Vec<_> = [(9., 12.), (12., 9.), (15., 12.)]
                .into_iter()
                .map(|(x, y)| rotate(x, y))
                .collect();
            path(&arrow);
            path(&[rotate(12., 9.), rotate(12., 15.)]);
        }
    }
}
pub fn scale(ui: &mut egui::Ui, value: &mut f64) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Scale").color(MUTED).size(12.));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(format!("{value:.2}×")).color(ACCENT));
        });
    });
    ui.horizontal(|ui| {
        let (r, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 36.),
            Sense::click_and_drag(),
        );
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
    });
}
