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

fn is_bright_color(c: Color32) -> bool {
    let luma = 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
    luma > 160.0
}

pub fn expected_body_font_size(font_scale_pct: u8) -> f32 {
    13.0 * (f32::from(font_scale_pct.clamp(85, 130)) / 100.0)
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

pub fn apply_theme(ctx: &egui::Context, t: &CustomThemeConfig, font_scale_pct: u8) {
    let mut style: Style = (*ctx.style()).clone();
    let scale = f32::from(font_scale_pct.clamp(85, 130)) / 100.0;
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(10.5 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(13.0 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(12.5 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(13.0 * scale),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(19.5 * scale),
    );
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
    let border_w = if t.is_high_contrast() { 1.5_f32 } else { 1.0_f32 };

    visuals.panel_fill = canvas;
    visuals.window_fill = canvas;
    visuals.extreme_bg_color = if t.dark_base {
        shift_color(t.canvas_bg, -6)
    } else {
        Color32::WHITE
    };
    visuals.faint_bg_color = card;

    visuals.widgets.noninteractive.bg_fill = card;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(border_w, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.inactive.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 10)
    } else {
        shift_color(t.card_bg, -18)
    };
    visuals.widgets.inactive.bg_stroke = Stroke::new(border_w, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.hovered.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 22)
    } else {
        shift_color(t.card_bg, -30)
    };
    visuals.widgets.hovered.bg_stroke = Stroke::new(border_w.max(1.2), accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.2_f32, text);

    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.bg_stroke = Stroke::new(border_w, accent);
    let active_text = if is_bright_color(accent) {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, active_text);

    visuals.widgets.open.bg_fill = if t.dark_base {
        shift_color(t.card_bg, 16)
    } else {
        shift_color(t.card_bg, -24)
    };
    visuals.widgets.open.bg_stroke = Stroke::new(border_w, accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(
        accent.r(),
        accent.g(),
        accent.b(),
        if t.is_high_contrast() {
            110
        } else if t.dark_base {
            78
        } else {
            52
        },
    );
    visuals.selection.stroke = Stroke::new(border_w.max(1.2), accent);

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
    let border_w = if t.is_high_contrast() { 1.5_f32 } else { 1.0_f32 };
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(border_w, rgb(t.card_border)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
}

pub fn compact_card(t: &CustomThemeConfig) -> egui::Frame {
    let border_w = if t.is_high_contrast() { 1.5_f32 } else { 1.0_f32 };
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(border_w, rgb(t.card_border)))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(10, 6))
}

pub fn section_title(ui: &mut egui::Ui, text: impl Into<String>, t: &CustomThemeConfig) {
    ui.label(
        egui::RichText::new(text.into())
            .strong()
            .color(accent_color(t)),
    );
}

fn button_palette(selected: bool, t: &CustomThemeConfig) -> (Color32, Stroke, Color32) {
    let accent = accent_color(t);
    let border_w = if t.is_high_contrast() { 1.5_f32 } else { 1.0_f32 };
    if selected {
        if t.is_high_contrast() {
            let fg = if is_bright_color(accent) {
                Color32::BLACK
            } else {
                Color32::WHITE
            };
            (accent, Stroke::new(2.0_f32, Color32::WHITE), fg)
        } else {
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
        }
    } else {
        (
            if t.dark_base {
                shift_color(t.card_bg, 8)
            } else {
                shift_color(t.card_bg, -12)
            },
            Stroke::new(border_w, rgb(t.card_border)),
            rgb(t.text_primary),
        )
    }
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
    let (fill, stroke, text_color) = button_palette(selected, t);
    let btn_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| f.size * 0.96)
        .unwrap_or(12.5);

    let mut rt = egui::RichText::new(label).size(btn_size).color(text_color);
    if selected {
        rt = rt.strong();
    }

    let btn = egui::Button::new(rt)
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8))
        .truncate();
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
        ui.add_sized([width, height], btn)
    })
    .inner
}

/// Compact pill-shaped button with crisp border for profile chips and mode toggles.
pub fn pill_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    t: &CustomThemeConfig,
) -> egui::Response {
    let (fill, stroke, text_color) = button_palette(selected, t);
    let btn_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| f.size * 0.96)
        .unwrap_or(12.5);

    let mut rt = egui::RichText::new(label).size(btn_size).color(text_color);
    if selected {
        rt = rt.strong();
    }

    let btn = egui::Button::new(rt)
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8));
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(9.0, 4.0);
        ui.add(btn)
    })
    .inner
}

pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 32);
    let inner = egui::Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).color(color).strong());
        });
    inner.response.interact(egui::Sense::hover())
}

/// Extra-compact pill badge for dense status strips in the 468px cockpit.
pub fn compact_status_pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
    let inner = egui::Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(0.9_f32, color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(10.5).color(color).strong());
        });
    inner.response.interact(egui::Sense::hover())
}

pub fn clickable_status_badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 34);
    let btn = egui::Button::new(egui::RichText::new(text).size(12.0).color(color).strong())
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(CornerRadius::same(12));
    ui.add(btn)
}

/// Compact pill badge with a custom-painted vector warning triangle (`▲`) + count.
pub fn clickable_warning_badge(
    ui: &mut egui::Ui,
    count: usize,
    color: Color32,
) -> egui::Response {
    let galley = egui::WidgetText::from(
        egui::RichText::new(count.to_string())
            .size(12.0)
            .color(color)
            .strong(),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );

    let pad_x = 9.0_f32;
    let tri_w = 12.0_f32;
    let gap = 5.0_f32;
    let width = (pad_x * 2.0 + tri_w + gap + galley.size().x).ceil();
    let height = 22.0_f32;

    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let fill_alpha = if resp.hovered() { 48 } else { 32 };
        let stroke_w = if resp.hovered() { 1.3_f32 } else { 1.0_f32 };
        let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), fill_alpha);

        painter.rect_filled(rect, CornerRadius::same(12), bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(stroke_w, color),
            egui::StrokeKind::Inside,
        );

        let cx = rect.left() + pad_x + tri_w * 0.5;
        let cy = rect.center().y;
        let top = egui::pos2(cx, cy - 5.2);
        let bot_right = egui::pos2(cx + 5.8, cy + 4.6);
        let bot_left = egui::pos2(cx - 5.8, cy + 4.6);
        let tri_fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 44);
        painter.add(egui::Shape::convex_polygon(
            vec![top, bot_right, bot_left],
            tri_fill,
            Stroke::new(1.4_f32, color),
        ));
        painter.line_segment(
            [egui::pos2(cx, cy - 1.8), egui::pos2(cx, cy + 1.2)],
            Stroke::new(1.4_f32, color),
        );
        painter.circle_filled(egui::pos2(cx, cy + 2.9), 0.85, color);

        let text_pos = egui::pos2(
            rect.left() + pad_x + tri_w + gap,
            cy - galley.size().y * 0.5,
        );
        painter.galley(text_pos, galley, color);
    }

    resp
}
