use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, Stroke, Style, Visuals};
use omvix_core::CustomThemeConfig;
use std::fs;
use std::path::Path;

pub const STATUS_OK: Color32 = Color32::from_rgb(34, 197, 94);
pub const STATUS_WARN: Color32 = Color32::from_rgb(245, 158, 11);
pub const STATUS_ERR: Color32 = Color32::from_rgb(239, 68, 68);

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn accent_color(t: &CustomThemeConfig) -> Color32 {
    rgb(t.accent)
}

pub fn sidebar_fill(t: &CustomThemeConfig) -> Color32 {
    rgb(t.sidebar_bg)
}

fn shift_color(c: [u8; 3], delta: i16) -> Color32 {
    let clamp = |v: u8| -> u8 { (v as i16 + delta).clamp(0, 255) as u8 };
    Color32::from_rgb(clamp(c[0]), clamp(c[1]), clamp(c[2]))
}

pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let candidate_paths = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];

    for path in candidate_paths {
        if Path::new(path).exists() {
            if let Ok(bytes) = fs::read(path) {
                fonts
                    .font_data
                    .insert("system_sans".to_owned(), FontData::from_owned(bytes).into());
                if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                    family.insert(0, "system_sans".to_owned());
                }
                break;
            }
        }
    }

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context, t: &CustomThemeConfig) {
    let mut style: Style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);

    let mut visuals = if t.dark_base {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    let accent = rgb(t.accent);
    let canvas = rgb(t.canvas_bg);
    let card = rgb(t.card_bg);
    let border = rgb(t.card_border);
    let text = rgb(t.text_primary);

    visuals.panel_fill = canvas;
    visuals.window_fill = canvas;
    visuals.extreme_bg_color = if t.dark_base {
        shift_color(t.canvas_bg, -6)
    } else {
        Color32::WHITE
    };
    visuals.faint_bg_color = card;

    visuals.widgets.noninteractive.bg_fill = card;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.inactive.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 10)
    } else {
        shift_color(t.card_bg, -18)
    };
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.hovered.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 22)
    } else {
        shift_color(t.card_bg, -30)
    };
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.2_f32, text);

    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
    let active_text = if t.dark_base { Color32::WHITE } else { Color32::WHITE };
    visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, active_text);

    visuals.widgets.open.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 16)
    } else {
        shift_color(t.card_bg, -24)
    };
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(
        accent.r(),
        accent.g(),
        accent.b(),
        if t.dark_base { 78 } else { 52 },
    );
    visuals.selection.stroke = Stroke::new(1.2_f32, accent);

    let r = CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = r;
    visuals.widgets.inactive.corner_radius = r;
    visuals.widgets.hovered.corner_radius = r;
    visuals.widgets.active.corner_radius = r;
    visuals.widgets.open.corner_radius = r;

    style.visuals = visuals;
    ctx.set_style(style);
}

pub fn card_frame(t: &CustomThemeConfig) -> egui::Frame {
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(1.0_f32, rgb(t.card_border)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
}

pub fn compact_card(t: &CustomThemeConfig) -> egui::Frame {
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(1.0_f32, rgb(t.card_border)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(11, 9))
}

pub fn section_title(ui: &mut egui::Ui, text: impl Into<String>, t: &CustomThemeConfig) {
    ui.label(
        egui::RichText::new(text.into())
            .strong()
            .color(accent_color(t)),
    );
}

/// Tactile segmented tile button inspired by G-Helper's rounded mode tiles.
pub fn tile_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    width: f32,
    height: f32,
    t: &CustomThemeConfig,
) -> egui::Response {
    let accent = accent_color(t);
    let (fill, stroke, text_color) = if selected {
        (
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                if t.dark_base { 54 } else { 38 },
            ),
            Stroke::new(1.3_f32, accent),
            if t.dark_base { Color32::WHITE } else { accent },
        )
    } else {
        (
            if t.dark_base {
                shift_color(t.card_bg, 8)
            } else {
                shift_color(t.card_bg, -12)
            },
            Stroke::new(1.0_f32, rgb(t.card_border)),
            rgb(t.text_primary),
        )
    };

    let mut rt = egui::RichText::new(label).size(13.0).color(text_color);
    if selected {
        rt = rt.strong();
    }

    let btn = egui::Button::new(rt)
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8));
    ui.add_sized([width, height], btn)
}

pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 32);
    egui::Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).color(color).strong());
        });
}

pub fn clickable_status_badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 34);
    let btn = egui::Button::new(egui::RichText::new(text).size(12.0).color(color).strong())
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(CornerRadius::same(12));
    ui.add(btn)
}
