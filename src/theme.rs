use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Visuals,
};

use crate::settings::ThemeMode;

/// The palette is intentionally restricted to black, white, gray and red.
/// Semantic colors (success/warning/danger/info) are expressed with neutral
/// grays or red shades instead of green/blue/orange, so the UI stays within the
/// four-color theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
}

impl Palette {
    pub fn new(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                dark: true,
                bg: Color32::from_rgb(8, 8, 9),
                panel: Color32::from_rgb(13, 13, 14),
                surface: Color32::from_rgb(18, 18, 20),
                surface_2: Color32::from_rgb(24, 24, 27),
                surface_3: Color32::from_rgb(32, 32, 35),
                border: Color32::from_rgb(46, 46, 50),
                border_strong: Color32::from_rgb(70, 70, 76),
                text: Color32::from_rgb(228, 228, 231),
                text_strong: Color32::from_rgb(250, 250, 251),
                muted: Color32::from_rgb(158, 158, 165),
                faint: Color32::from_rgb(102, 102, 110),
                accent: Color32::from_rgb(225, 29, 72),
                accent_hover: Color32::from_rgb(251, 113, 133),
                accent_soft: Color32::from_rgba_unmultiplied(225, 29, 72, 30),
                on_accent: Color32::from_rgb(255, 255, 255),
                success: Color32::from_rgb(228, 228, 231),
                warning: Color32::from_rgb(251, 113, 133),
                danger: Color32::from_rgb(239, 68, 68),
                info: Color32::from_rgb(161, 161, 170),
            },
            ThemeMode::Light => Self {
                dark: false,
                bg: Color32::from_rgb(247, 247, 248),
                panel: Color32::from_rgb(252, 252, 252),
                surface: Color32::from_rgb(255, 255, 255),
                surface_2: Color32::from_rgb(248, 248, 249),
                surface_3: Color32::from_rgb(240, 240, 241),
                border: Color32::from_rgb(224, 224, 227),
                border_strong: Color32::from_rgb(197, 197, 202),
                text: Color32::from_rgb(39, 39, 42),
                text_strong: Color32::from_rgb(15, 15, 17),
                muted: Color32::from_rgb(100, 100, 108),
                faint: Color32::from_rgb(145, 145, 153),
                accent: Color32::from_rgb(190, 18, 60),
                accent_hover: Color32::from_rgb(159, 18, 57),
                accent_soft: Color32::from_rgba_unmultiplied(190, 18, 60, 22),
                on_accent: Color32::from_rgb(255, 255, 255),
                success: Color32::from_rgb(24, 24, 27),
                warning: Color32::from_rgb(190, 18, 60),
                danger: Color32::from_rgb(153, 27, 27),
                info: Color32::from_rgb(82, 82, 91),
            },
        }
    }

    pub fn card(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.surface)
            .stroke(Stroke::new(1.0_f32, self.border))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(18))
    }

    pub fn soft_card(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.surface_2)
            .stroke(Stroke::new(1.0_f32, self.border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
    }

    pub fn accent_tint(&self, alpha: u8) -> Color32 {
        let [r, g, b, _] = self.accent.to_array();
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    }
}

pub fn provider_color(id: &str) -> Color32 {
    match id {
        "rsproxy" => Color32::from_rgb(225, 29, 72),
        "ustc" => Color32::from_rgb(161, 161, 170),
        "tuna" => Color32::from_rgb(113, 113, 122),
        "sjtu" => Color32::from_rgb(251, 113, 133),
        "aliyun" => Color32::from_rgb(244, 63, 94),
        "huawei" => Color32::from_rgb(113, 113, 122),
        "official" => Color32::from_rgb(161, 161, 170),
        _ => Color32::from_rgb(113, 113, 122),
    }
}

/// Install a CJK-capable font. Returns the path that was loaded, if any.
pub fn install_fonts(ctx: &egui::Context) -> Option<String> {
    #[cfg(windows)]
    let candidates = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\Deng.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ];
    #[cfg(target_os = "macos")]
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    let candidates = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ];

    for path in candidates {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if ttf_parser::Face::parse(&bytes, 0).is_err() {
            continue;
        }
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("cjk".to_owned(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return Some(path.to_owned());
    }
    None
}

pub fn apply_style(ctx: &egui::Context, mode: ThemeMode) {
    let palette = Palette::new(mode);
    ctx.set_theme(match mode {
        ThemeMode::Dark => egui::ThemePreference::Dark,
        ThemeMode::Light => egui::ThemePreference::Light,
    });

    let theme = match mode {
        ThemeMode::Dark => egui::Theme::Dark,
        ThemeMode::Light => egui::Theme::Light,
    };
    let mut style = (*ctx.style_of(theme)).clone();
    let mut visuals = match mode {
        ThemeMode::Dark => Visuals::dark(),
        ThemeMode::Light => Visuals::light(),
    };
    visuals.panel_fill = palette.bg;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.surface_2;
    visuals.faint_bg_color = palette.surface_2;
    visuals.code_bg_color = palette.surface_3;
    visuals.text_edit_bg_color = Some(palette.surface_2);
    visuals.window_stroke = Stroke::new(1.0_f32, palette.border);
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.danger;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.muted);
    visuals.selection.bg_fill = palette.accent_tint(70);
    visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);

    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface_2;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

    visuals.widgets.inactive.bg_fill = palette.surface_2;
    visuals.widgets.inactive.weak_bg_fill = palette.surface_2;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, palette.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

    visuals.widgets.hovered.bg_fill = palette.surface_3;
    visuals.widgets.hovered.weak_bg_fill = palette.surface_3;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, palette.accent_hover);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, palette.text_strong);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

    visuals.widgets.active.bg_fill = palette.accent_soft;
    visuals.widgets.active.weak_bg_fill = palette.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, palette.text_strong);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);

    visuals.widgets.open.bg_fill = palette.surface_3;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, palette.border_strong);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.open.corner_radius = CornerRadius::same(8);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.slider_width = 180.0;
    style.spacing.combo_width = 180.0;
    style.spacing.indent = 18.0;
    style.animation_time = 0.14;

    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(25.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(11.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(12.5, FontFamily::Monospace),
    );
    ctx.set_style_of(theme, style);
}
