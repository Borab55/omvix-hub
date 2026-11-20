use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, Stroke, Style, Visuals};
use std::fs;
use std::path::Path;

pub const ACCENT_CYAN: Color32 = Color32::from_rgb(0, 180, 216);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(2, 132, 199);
pub const STATUS_OK: Color32 = Color32::from_rgb(34, 197, 94);
pub const STATUS_WARN: Color32 = Color32::from_rgb(245, 158, 11);
pub const STATUS_ERR: Color32 = Color32::from_rgb(239, 68, 68);

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

pub fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut style: Style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);

    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    if dark {
        visuals.panel_fill = Color32::from_rgb(15, 20, 27);
        visuals.window_fill = Color32::from_rgb(15, 20, 27);
        visuals.extreme_bg_color = Color32::from_rgb(10, 14, 20);
        visuals.faint_bg_color = Color32::from_rgb(22, 29, 39);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(22, 29, 39);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(38, 49, 64));
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 39, 54);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(41, 54, 74);
        visuals.widgets.active.bg_fill = Color32::from_rgb(0, 140, 175);
        visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0, 180, 216, 85);
        visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_CYAN);
    } else {
        visuals.panel_fill = Color32::from_rgb(244, 246, 249);
        visuals.window_fill = Color32::from_rgb(244, 246, 249);
        visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
        visuals.faint_bg_color = Color32::from_rgb(233, 238, 245);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(255, 255, 255);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(218, 224, 233));
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(229, 234, 242);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(212, 221, 235);
        visuals.widgets.active.bg_fill = ACCENT_LIGHT;
        visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(2, 132, 199, 55);
        visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_LIGHT);
    }

    let r = CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = r;
    visuals.widgets.inactive.corner_radius = r;
    visuals.widgets.hovered.corner_radius = r;
    visuals.widgets.active.corner_radius = r;

    style.visuals = visuals;
    ctx.set_style(style);
}

pub fn card_frame(dark: bool) -> egui::Frame {
    let (fill, stroke) = if dark {
        (
            Color32::from_rgb(22, 29, 39),
            Stroke::new(1.0_f32, Color32::from_rgb(38, 49, 64)),
        )
    } else {
        (
            Color32::from_rgb(255, 255, 255),
            Stroke::new(1.0_f32, Color32::from_rgb(216, 223, 232)),
        )
    };

    egui::Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
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
