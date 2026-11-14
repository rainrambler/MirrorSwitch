use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Response, RichText, Sense,
    Stroke, Ui, Vec2,
};

use crate::theme::Palette;

#[derive(Clone, Copy, Debug)]
pub enum NavIcon {
    Overview,
    Mirrors,
    Speed,
    Backup,
    Settings,
}

pub fn nav_item(
    ui: &mut Ui,
    palette: Palette,
    icon: NavIcon,
    label: &str,
    selected: bool,
) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 44.0), Sense::click());
    let hovered = response.hovered();
    let bg = if selected {
        palette.accent_soft
    } else if hovered {
        palette.surface_2
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - 11.0),
            Vec2::new(3.0, 22.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), palette.accent);
    }
    let icon_center = egui::pos2(rect.left() + 26.0, rect.center().y);
    paint_nav_icon(
        ui.painter(),
        icon_center,
        icon,
        if selected || hovered {
            palette.accent
        } else {
            palette.muted
        },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 48.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.5),
        if selected {
            palette.text_strong
        } else if hovered {
            palette.text
        } else {
            palette.muted
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn paint_nav_icon(painter: &egui::Painter, center: egui::Pos2, icon: NavIcon, color: Color32) {
    let stroke = Stroke::new(1.6_f32, color);
    let s = 7.5;
    match icon {
        NavIcon::Overview => {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let r = egui::Rect::from_center_size(
                    center + Vec2::new(dx * s * 0.62, dy * s * 0.62),
                    Vec2::splat(s * 0.9),
                );
                painter.rect_filled(r, CornerRadius::same(2), color);
            }
        }
        NavIcon::Mirrors => {
            let y1 = center.y - 4.0;
            let y2 = center.y + 4.0;
            painter.line_segment(
                [
                    egui::pos2(center.x - 7.0, y1),
                    egui::pos2(center.x + 6.0, y1),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 6.0, y2),
                    egui::pos2(center.x + 7.0, y2),
                ],
                stroke,
            );
            arrow_head(painter, egui::pos2(center.x + 7.0, y1), 1.0, color);
            arrow_head(painter, egui::pos2(center.x - 7.0, y2), -1.0, color);
        }
        NavIcon::Speed => {
            let radius = 8.0;
            let mut points = Vec::new();
            for i in 0..=24 {
                let angle = std::f32::consts::PI + (i as f32 / 24.0) * std::f32::consts::PI;
                points.push(center + Vec2::angled(angle) * radius);
            }
            painter.add(egui::Shape::line(points, stroke));
            painter.line_segment(
                [center, center + Vec2::new(5.0, -5.0)],
                Stroke::new(1.8_f32, color),
            );
            painter.circle_filled(center, 1.8, color);
        }
        NavIcon::Backup => {
            let body =
                egui::Rect::from_center_size(center + Vec2::new(0.0, 2.0), Vec2::new(16.0, 11.0));
            painter.rect_filled(body, CornerRadius::same(2), color);
            let lid =
                egui::Rect::from_center_size(center + Vec2::new(0.0, -5.5), Vec2::new(17.0, 3.0));
            painter.rect_filled(lid, CornerRadius::same(1), color);
            painter.rect_filled(
                egui::Rect::from_center_size(center + Vec2::new(0.0, 2.0), Vec2::new(4.0, 2.0)),
                CornerRadius::same(1),
                palette_contrast(color),
            );
        }
        NavIcon::Settings => {
            for (index, dy) in [-6.0, 0.0, 6.0].iter().enumerate() {
                let y = center.y + dy;
                painter.line_segment(
                    [egui::pos2(center.x - 8.0, y), egui::pos2(center.x + 8.0, y)],
                    stroke,
                );
                let knob_x = center.x + if index % 2 == 0 { 3.0 } else { -3.0 };
                painter.circle_filled(egui::pos2(knob_x, y), 2.1, color);
            }
        }
    }
}

fn arrow_head(painter: &egui::Painter, tip: egui::Pos2, direction: f32, color: Color32) {
    let size = 3.2;
    let points = vec![
        tip,
        tip + Vec2::new(-size * direction, -size),
        tip + Vec2::new(-size * direction, size),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn palette_contrast(color: Color32) -> Color32 {
    if color.r() as u16 + color.g() as u16 + color.b() as u16 > 380 {
        Color32::from_rgb(15, 15, 17)
    } else {
        Color32::from_rgb(245, 245, 246)
    }
}

pub fn paint_logo(ui: &Ui, rect: egui::Rect, palette: Palette) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), palette.accent);
    let center = rect.center();
    let arrow = Stroke::new(2.2_f32, palette.on_accent);
    painter.line_segment(
        [
            egui::pos2(center.x - 8.0, center.y - 4.0),
            egui::pos2(center.x + 7.0, center.y - 4.0),
        ],
        arrow,
    );
    painter.line_segment(
        [
            egui::pos2(center.x - 7.0, center.y + 4.0),
            egui::pos2(center.x + 8.0, center.y + 4.0),
        ],
        arrow,
    );
    arrow_head(
        painter,
        egui::pos2(center.x + 8.0, center.y - 4.0),
        1.0,
        palette.on_accent,
    );
    arrow_head(
        painter,
        egui::pos2(center.x - 8.0, center.y + 4.0),
        -1.0,
        palette.on_accent,
    );
    painter.circle_filled(
        egui::pos2(rect.right() - 9.0, rect.bottom() - 9.0),
        3.0,
        palette.on_accent,
    );
}

pub fn chip(ui: &mut Ui, text: impl Into<String>, fg: Color32, bg: Color32) -> Response {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(11.0).color(fg));
        })
        .response
}

pub fn dot(ui: &mut Ui, color: Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
}

pub fn primary_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette_of(ui);
    let button = egui::Button::new(RichText::new(text.into()).color(palette.on_accent).strong())
        .fill(palette.accent)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(9))
        .min_size(Vec2::new(0.0, 35.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn secondary_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette_of(ui);
    let button = egui::Button::new(RichText::new(text.into()).color(palette.text))
        .fill(palette.surface_2)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(CornerRadius::same(9))
        .min_size(Vec2::new(0.0, 35.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn ghost_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette_of(ui);
    let button = egui::Button::new(RichText::new(text.into()).color(palette.muted))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 30.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn danger_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let palette = palette_of(ui);
    let button = egui::Button::new(RichText::new(text.into()).color(palette.danger).strong())
        .fill(palette.surface_2)
        .stroke(Stroke::new(1.0_f32, palette.danger.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(9))
        .min_size(Vec2::new(0.0, 35.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn palette_of(ui: &Ui) -> Palette {
    // The palette is deterministic from a single visual signal.
    if ui.visuals().dark_mode {
        Palette::new(crate::settings::ThemeMode::Dark)
    } else {
        Palette::new(crate::settings::ThemeMode::Light)
    }
}

pub fn page_header(
    ui: &mut Ui,
    palette: Palette,
    title: &str,
    subtitle: &str,
    add_right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(
                RichText::new(title)
                    .size(25.0)
                    .color(palette.text_strong)
                    .strong(),
            );
            ui.add_space(2.0);
            ui.label(RichText::new(subtitle).size(12.5).color(palette.muted));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            add_right(ui);
        });
    });
    ui.add_space(14.0);
}

pub fn section_title(ui: &mut Ui, palette: Palette, title: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .size(15.0)
                .color(palette.text_strong)
                .strong(),
        );
        if let Some(hint) = hint {
            ui.label(RichText::new(hint).size(11.5).color(palette.faint));
        }
    });
    ui.add_space(6.0);
}

pub fn metric_bar(ui: &mut Ui, palette: Palette, fraction: f32, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), palette.surface_3);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled = egui::Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * fraction).max(3.0), rect.height()),
        );
        ui.painter()
            .rect_filled(filled, CornerRadius::same(3), color);
    }
}

pub fn empty_state(ui: &mut Ui, palette: Palette, title: &str, description: &str, height: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        Layout::top_down(Align::Center),
        |ui| {
            ui.add_space((height - 58.0).max(8.0) / 2.0);
            ui.label(RichText::new(title).size(14.0).color(palette.text).strong());
            ui.add_space(3.0);
            ui.label(RichText::new(description).size(12.0).color(palette.muted));
        },
    );
}

pub fn notice(ui: &mut Ui, palette: Palette, color: Color32, title: &str, body: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(if palette.dark { 0.14 } else { 0.08 }))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(11))
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                dot(ui, color, 3.5);
                ui.label(RichText::new(title).size(12.5).color(color).strong());
                ui.label(RichText::new(body).size(12.0).color(palette.text));
            });
        });
}

pub fn toggle(ui: &mut Ui, palette: Palette, value: &mut bool, label: &str) -> Response {
    let width = ui.available_width().max(220.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 9.0),
        Vec2::new(36.0, 18.0),
    );
    let track_color = if *value {
        palette.accent
    } else {
        palette.surface_3
    };
    ui.painter()
        .rect_filled(track, CornerRadius::same(9), track_color);
    let knob_x = if *value {
        track.right() - 9.0
    } else {
        track.left() + 9.0
    };
    ui.painter().circle_filled(
        egui::pos2(knob_x, track.center().y),
        7.0,
        if *value {
            palette.on_accent
        } else {
            palette.muted
        },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 48.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        palette.text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn theme_toggle(ui: &mut Ui, palette: Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(10),
        if hovered {
            palette.surface_3
        } else {
            palette.surface_2
        },
    );
    let center = rect.center();
    if palette.dark {
        // Moon: a filled circle with an offset cut-out.
        ui.painter()
            .circle_filled(center + Vec2::new(-1.0, -1.0), 7.0, palette.accent);
        ui.painter()
            .circle_filled(center + Vec2::new(3.0, -3.0), 6.2, palette.surface_2);
    } else {
        ui.painter().circle_filled(center, 5.2, palette.warning);
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            ui.painter().line_segment(
                [
                    center + Vec2::angled(angle) * 8.4,
                    center + Vec2::angled(angle) * 11.0,
                ],
                Stroke::new(1.5_f32, palette.warning),
            );
        }
    }
    response
        .on_hover_text(if palette.dark {
            "切换到浅色主题"
        } else {
            "切换到深色主题"
        })
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
