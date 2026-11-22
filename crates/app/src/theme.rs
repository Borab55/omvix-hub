use egui::{
    ecolor::{Hsva, HsvaGamma},
    lerp, pos2, remap_clamp, vec2, Area, Color32, CornerRadius, DragValue, FontData,
    FontDefinitions, FontFamily, Frame, Key, Margin, Mesh, Order, Response, Sense, Shape, Stroke,
    StrokeKind, Style, TextWrapMode, Ui, UiKind, Vec2, Visuals,
};
use omvix_core::{CustomThemeConfig, RgbColor};
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
    let border_w = if t.is_high_contrast() {
        1.5_f32
    } else {
        1.0_f32
    };

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
    let border_w = if t.is_high_contrast() {
        1.5_f32
    } else {
        1.0_f32
    };
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(border_w, rgb(t.card_border)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
}

pub fn compact_card(t: &CustomThemeConfig) -> egui::Frame {
    let border_w = if t.is_high_contrast() {
        1.5_f32
    } else {
        1.0_f32
    };
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
    let border_w = if t.is_high_contrast() {
        1.5_f32
    } else {
        1.0_f32
    };
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
    pill_button_enabled(ui, label, selected, true, t)
}

/// Interactive pill button supporting disabled states without collapsing layout or wrapping text vertically.
pub fn pill_button_enabled(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
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
        .wrap_mode(TextWrapMode::Extend);
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(10.0, 4.0);
        ui.add_enabled(enabled, btn)
    })
    .inner
}

fn render_toggle_button(
    ui: &mut Ui,
    label: &str,
    active: &mut bool,
    active_tag: &str,
    header_emphasis: bool,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let text_base = rgb(t.text_primary);
    let text_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| {
            if header_emphasis {
                f.size
            } else {
                f.size * 0.94
            }
        })
        .unwrap_or(12.5);

    let (fill, stroke, label_color) = if *active {
        let bg = if t.is_high_contrast() {
            accent
        } else {
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                if t.dark_base { 52 } else { 36 },
            )
        };
        let fg = if t.is_high_contrast() {
            if is_bright_color(accent) {
                Color32::BLACK
            } else {
                Color32::WHITE
            }
        } else if t.dark_base {
            Color32::WHITE
        } else {
            accent
        };
        (bg, Stroke::new(1.3_f32, accent), fg)
    } else {
        let bg = if t.dark_base {
            shift_color(t.card_bg, 6)
        } else {
            shift_color(t.card_bg, -10)
        };
        let muted = Color32::from_rgba_unmultiplied(
            text_base.r(),
            text_base.g(),
            text_base.b(),
            if t.dark_base { 165 } else { 155 },
        );
        (bg, Stroke::new(1.0_f32, rgb(t.card_border)), muted)
    };

    let display_text = if *active {
        format!("{label}  •  {active_tag}")
    } else {
        label.to_string()
    };

    let mut rt = egui::RichText::new(display_text)
        .size(text_size)
        .color(label_color);
    if *active || header_emphasis {
        rt = rt.strong();
    }

    let btn = egui::Button::new(rt)
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8))
        .wrap_mode(TextWrapMode::Extend);

    let mut resp = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = vec2(9.0, 4.0);
            ui.add(btn)
        })
        .inner;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *active = !*active;
        resp.mark_changed();
    }
    resp
}

/// Illuminated text-button toggle for feature options (lights up with `active_tag` when active,
/// dims subtly with tag hidden when off).
pub fn feature_toggle_button(
    ui: &mut Ui,
    label: &str,
    active: &mut bool,
    active_tag: &str,
    t: &CustomThemeConfig,
) -> Response {
    render_toggle_button(ui, label, active, active_tag, false, t)
}

/// Section-header variant of `feature_toggle_button` with stronger header typography.
pub fn section_toggle_button(
    ui: &mut Ui,
    title: &str,
    active: &mut bool,
    active_tag: &str,
    t: &CustomThemeConfig,
) -> Response {
    render_toggle_button(ui, title, active, active_tag, true, t)
}

/// Circular cockpit dial button that opens the corresponding side-drawer detail view.
pub fn detail_dial_button(
    ui: &mut Ui,
    active: bool,
    tooltip: &str,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let size = Vec2::splat(22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fill = if active {
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                if t.dark_base { 66 } else { 44 },
            )
        } else if resp.hovered() {
            if t.dark_base {
                shift_color(t.card_bg, 22)
            } else {
                shift_color(t.card_bg, -24)
            }
        } else if t.dark_base {
            shift_color(t.card_bg, 10)
        } else {
            shift_color(t.card_bg, -12)
        };
        let stroke_color = if active || resp.hovered() {
            accent
        } else {
            rgb(t.card_border)
        };
        let fg = if active || resp.hovered() {
            if t.dark_base {
                Color32::WHITE
            } else {
                accent
            }
        } else {
            rgb(t.text_primary)
        };

        let center = rect.center();
        let radius = rect.width() * 0.5 - 0.8;
        p.circle_filled(center, radius, fill);
        p.circle_stroke(center, radius, Stroke::new(1.1_f32, stroke_color));

        // Subtle left-opening drawer chevron (`«`) + dot inside dial
        let tip_x = center.x - 2.6;
        let back_x = center.x + 1.8;
        let dy = 3.4_f32;
        let line_stroke = Stroke::new(1.5_f32, fg);
        p.line_segment(
            [pos2(back_x, center.y - dy), pos2(tip_x, center.y)],
            line_stroke,
        );
        p.line_segment(
            [pos2(tip_x, center.y), pos2(back_x, center.y + dy)],
            line_stroke,
        );
        p.circle_filled(pos2(center.x + 4.4, center.y), 1.15, accent);
    }

    resp.on_hover_text(tooltip)
}

/// Sleek rounded keyboard lighting swatch showing either 1 solid color or 4 vertical RGB zone bars.
pub fn lighting_swatch_button(
    ui: &mut Ui,
    zones: &[RgbColor; 4],
    selected: bool,
    tooltip: &str,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let size = Vec2::splat(22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let inner = rect.shrink(2.2);
        let solid = zones.iter().all(|z| *z == zones[0]);
        if solid {
            let c = Color32::from_rgb(zones[0].r, zones[0].g, zones[0].b);
            p.rect_filled(inner, CornerRadius::same(4), c);
        } else {
            let strip_w = inner.width() / 4.0;
            for (idx, z) in zones.iter().enumerate() {
                let x0 = inner.left() + idx as f32 * strip_w;
                let strip = egui::Rect::from_min_max(
                    pos2(x0, inner.top()),
                    pos2(x0 + strip_w, inner.bottom()),
                );
                let cr = match idx {
                    0 => CornerRadius {
                        nw: 4,
                        sw: 4,
                        ne: 0,
                        se: 0,
                    },
                    3 => CornerRadius {
                        nw: 0,
                        sw: 0,
                        ne: 4,
                        se: 4,
                    },
                    _ => CornerRadius::ZERO,
                };
                p.rect_filled(strip, cr, Color32::from_rgb(z.r, z.g, z.b));
            }
        }

        // Crisp subtle top sheen & outer active/hover ring
        let inner_ring = if selected {
            Color32::WHITE
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 40)
        };
        p.rect_stroke(
            inner,
            CornerRadius::same(4),
            Stroke::new(1.0_f32, inner_ring),
            StrokeKind::Inside,
        );

        let outer_stroke = if selected {
            Stroke::new(2.0_f32, accent)
        } else if resp.hovered() {
            Stroke::new(1.4_f32, accent)
        } else {
            Stroke::new(1.0_f32, rgb(t.card_border))
        };
        p.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(6),
            outer_stroke,
            StrokeKind::Inside,
        );
    }

    resp.on_hover_text(tooltip)
}

#[derive(Debug, Clone, Copy)]
pub enum ColorButtonStyle {
    Swatch { size: Vec2, radius: u8 },
    ColorWheel { size: f32 },
}

fn paint_hsv_wheel_button(
    ui: &Ui,
    rect: egui::Rect,
    current_rgb: [u8; 3],
    open: bool,
    hovered: bool,
    t: &CustomThemeConfig,
) {
    let p = ui.painter_at(rect);
    let center = rect.center();
    let outer_r = rect.width().min(rect.height()) * 0.5 - 1.2;
    let ring_r = (outer_r - 2.6).max(3.0);
    const SEGMENTS: usize = 24;
    for i in 0..SEGMENTS {
        let a0 = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let hue = i as f32 / SEGMENTS as f32;
        let c: Color32 = Hsva::new(hue, 0.92, 0.96, 1.0).into();
        let p0 = pos2(center.x + ring_r * a0.cos(), center.y + ring_r * a0.sin());
        let p1 = pos2(center.x + ring_r * a1.cos(), center.y + ring_r * a1.sin());
        p.line_segment([p0, p1], Stroke::new(3.4_f32, c));
    }
    let dot_r = (outer_r - 5.2).max(3.0);
    let cur = Color32::from_rgb(current_rgb[0], current_rgb[1], current_rgb[2]);
    p.circle_filled(center, dot_r, cur);
    p.circle_stroke(
        center,
        dot_r,
        Stroke::new(
            1.0_f32,
            if open || hovered {
                Color32::WHITE
            } else {
                rgb(t.card_border)
            },
        ),
    );
    if open || hovered {
        p.circle_stroke(center, outer_r, Stroke::new(1.2_f32, accent_color(t)));
    }
}

/// Fixed-width color picker popup that never expands horizontally when RGB values reach 3 digits.
pub fn fixed_color_picker_button(
    ui: &mut Ui,
    id_source: impl std::hash::Hash,
    srgb: &mut [u8; 3],
    style: ColorButtonStyle,
    t: &CustomThemeConfig,
) -> Response {
    let base_id = ui.make_persistent_id(id_source);
    let popup_id = base_id.with("fixed_color_popup");
    let hsv_cache_id = base_id.with("fixed_color_hsv");
    let hex_buf_id = base_id.with("fixed_color_hex_buf");
    let hex_src_id = base_id.with("fixed_color_hex_src");
    let open = ui.memory(|m| m.is_popup_open(popup_id));

    let alloc_size = match style {
        ColorButtonStyle::Swatch { size, .. } => size,
        ColorButtonStyle::ColorWheel { size } => Vec2::splat(size),
    };
    let (rect, mut button_response) = ui.allocate_exact_size(alloc_size, Sense::click());
    if button_response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        match style {
            ColorButtonStyle::Swatch { radius, .. } => {
                let p = ui.painter_at(rect);
                let fill = Color32::from_rgb(srgb[0], srgb[1], srgb[2]);
                let border_c = if open || button_response.hovered() {
                    accent_color(t)
                } else {
                    rgb(t.card_border)
                };
                p.rect_filled(rect.shrink(1.0), CornerRadius::same(radius), fill);
                p.rect_stroke(
                    rect.shrink(0.5),
                    CornerRadius::same(radius),
                    Stroke::new(if open { 1.8_f32 } else { 1.2_f32 }, border_c),
                    StrokeKind::Inside,
                );
            }
            ColorButtonStyle::ColorWheel { .. } => {
                paint_hsv_wheel_button(ui, rect, *srgb, open, button_response.hovered(), t);
            }
        }
    }

    if button_response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }

    if ui.memory(|m| m.is_popup_open(popup_id)) {
        const POPUP_INNER_W: f32 = 248.0;
        let screen = ui.ctx().screen_rect().shrink(6.0);
        let est_outer_w = POPUP_INNER_W + 20.0;
        let est_outer_h = 246.0;
        let px = rect
            .left()
            .min(screen.right() - est_outer_w)
            .max(screen.left());
        let py = if rect.bottom() + est_outer_h <= screen.bottom() {
            rect.bottom() + 4.0
        } else {
            (rect.top() - est_outer_h - 4.0).max(screen.top())
        };

        let mut hsvag: HsvaGamma = ui
            .ctx()
            .data_mut(|d| d.get_temp::<HsvaGamma>(hsv_cache_id))
            .unwrap_or_else(|| HsvaGamma::from(Color32::from_rgb(srgb[0], srgb[1], srgb[2])));
        let cached_rgb = Color32::from(Hsva::from(hsvag));
        if [cached_rgb.r(), cached_rgb.g(), cached_rgb.b()] != *srgb {
            hsvag = HsvaGamma::from(Color32::from_rgb(srgb[0], srgb[1], srgb[2]));
        }

        let area_response = Area::new(popup_id)
            .kind(UiKind::Picker)
            .order(Order::Foreground)
            .fixed_pos(pos2(px, py))
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(POPUP_INNER_W);
                    ui.set_max_width(POPUP_INNER_W);
                    ui.spacing_mut().item_spacing = vec2(7.0, 6.0);

                    let mut edited = false;

                    // 1. 2D Saturation / Value square (fixed 248x132)
                    let (sv_rect, sv_resp) =
                        ui.allocate_exact_size(vec2(POPUP_INNER_W, 132.0), Sense::click_and_drag());
                    if let Some(mpos) = sv_resp.interact_pointer_pos() {
                        hsvag.s = remap_clamp(mpos.x, sv_rect.left()..=sv_rect.right(), 0.0..=1.0);
                        hsvag.v = remap_clamp(mpos.y, sv_rect.bottom()..=sv_rect.top(), 0.0..=1.0);
                        edited = true;
                    }
                    if ui.is_rect_visible(sv_rect) {
                        const STEPS: u32 = 24;
                        let mut mesh = Mesh::default();
                        for xi in 0..=STEPS {
                            for yi in 0..=STEPS {
                                let xt = xi as f32 / STEPS as f32;
                                let yt = yi as f32 / STEPS as f32;
                                let c: Color32 = HsvaGamma {
                                    h: hsvag.h,
                                    s: xt,
                                    v: yt,
                                    a: 1.0,
                                }
                                .into();
                                let vx = lerp(sv_rect.left()..=sv_rect.right(), xt);
                                let vy = lerp(sv_rect.bottom()..=sv_rect.top(), yt);
                                mesh.colored_vertex(pos2(vx, vy), c);
                                if xi < STEPS && yi < STEPS {
                                    let row = STEPS + 1;
                                    let i00 = xi * row + yi;
                                    mesh.add_triangle(i00, i00 + 1, i00 + row);
                                    mesh.add_triangle(i00 + 1, i00 + row, i00 + row + 1);
                                }
                            }
                        }
                        ui.painter().add(Shape::mesh(mesh));
                        ui.painter().rect_stroke(
                            sv_rect,
                            CornerRadius::same(4),
                            Stroke::new(1.0_f32, rgb(t.card_border)),
                            StrokeKind::Inside,
                        );
                        let hx = lerp(sv_rect.left()..=sv_rect.right(), hsvag.s);
                        let hy = lerp(sv_rect.bottom()..=sv_rect.top(), hsvag.v);
                        let cur_sv: Color32 = hsvag.into();
                        let ring = if is_bright_color(cur_sv) {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        };
                        ui.painter()
                            .circle_stroke(pos2(hx, hy), 5.5, Stroke::new(1.8_f32, ring));
                    }

                    // 2. 1D Rainbow Hue bar (fixed 248x16)
                    let (hue_rect, hue_resp) =
                        ui.allocate_exact_size(vec2(POPUP_INNER_W, 16.0), Sense::click_and_drag());
                    if let Some(mpos) = hue_resp.interact_pointer_pos() {
                        hsvag.h =
                            remap_clamp(mpos.x, hue_rect.left()..=hue_rect.right(), 0.0..=1.0);
                        edited = true;
                    }
                    if ui.is_rect_visible(hue_rect) {
                        const HUE_SEGMENTS: u32 = 36;
                        let mut mesh = Mesh::default();
                        for i in 0..=HUE_SEGMENTS {
                            let tt = i as f32 / HUE_SEGMENTS as f32;
                            let c: Color32 = HsvaGamma {
                                h: tt,
                                s: 1.0,
                                v: 1.0,
                                a: 1.0,
                            }
                            .into();
                            let vx = lerp(hue_rect.left()..=hue_rect.right(), tt);
                            mesh.colored_vertex(pos2(vx, hue_rect.top()), c);
                            mesh.colored_vertex(pos2(vx, hue_rect.bottom()), c);
                            if i < HUE_SEGMENTS {
                                mesh.add_triangle(2 * i, 2 * i + 1, 2 * i + 2);
                                mesh.add_triangle(2 * i + 1, 2 * i + 2, 2 * i + 3);
                            }
                        }
                        ui.painter().add(Shape::mesh(mesh));
                        ui.painter().rect_stroke(
                            hue_rect,
                            CornerRadius::same(3),
                            Stroke::new(1.0_f32, rgb(t.card_border)),
                            StrokeKind::Inside,
                        );
                        let hx = lerp(hue_rect.left()..=hue_rect.right(), hsvag.h);
                        ui.painter().line_segment(
                            [pos2(hx, hue_rect.top()), pos2(hx, hue_rect.bottom())],
                            Stroke::new(2.2_f32, Color32::WHITE),
                        );
                    }

                    // Sync HSV -> RGB before numeric & hex rows
                    let cur_color = Color32::from(Hsva::from(hsvag));
                    let mut rgb_edit = [cur_color.r(), cur_color.g(), cur_color.b()];

                    // 3. Fixed 3-column RGB DragValues (78*3 + 7*2 = 248px: zero jump at 3 digits!)
                    let mut rgb_edited = false;
                    ui.horizontal(|ui| {
                        ui.set_width(POPUP_INNER_W);
                        ui.spacing_mut().item_spacing.x = 7.0;
                        let col_w = 78.0;
                        rgb_edited |= ui
                            .add_sized(
                                [col_w, 22.0],
                                DragValue::new(&mut rgb_edit[0]).speed(0.5).prefix("R "),
                            )
                            .changed();
                        rgb_edited |= ui
                            .add_sized(
                                [col_w, 22.0],
                                DragValue::new(&mut rgb_edit[1]).speed(0.5).prefix("G "),
                            )
                            .changed();
                        rgb_edited |= ui
                            .add_sized(
                                [col_w, 22.0],
                                DragValue::new(&mut rgb_edit[2]).speed(0.5).prefix("B "),
                            )
                            .changed();
                    });
                    if rgb_edited {
                        hsvag = HsvaGamma::from(Color32::from_rgb(
                            rgb_edit[0],
                            rgb_edit[1],
                            rgb_edit[2],
                        ));
                        edited = true;
                    }

                    // 4. Fixed preview swatch + #RRGGBB hex editor
                    let formatted_hex =
                        format!("#{:02X}{:02X}{:02X}", rgb_edit[0], rgb_edit[1], rgb_edit[2]);
                    let last_synced_rgb = ui.ctx().data_mut(|d| d.get_temp::<[u8; 3]>(hex_src_id));
                    let mut hex_str = if last_synced_rgb == Some(rgb_edit) {
                        ui.ctx()
                            .data_mut(|d| d.get_temp::<String>(hex_buf_id))
                            .unwrap_or_else(|| formatted_hex.clone())
                    } else {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(hex_src_id, rgb_edit);
                            d.insert_temp(hex_buf_id, formatted_hex.clone());
                        });
                        formatted_hex
                    };

                    ui.horizontal(|ui| {
                        ui.set_width(POPUP_INNER_W);
                        let (prev_rect, _) =
                            ui.allocate_exact_size(vec2(52.0, 22.0), Sense::hover());
                        ui.painter().rect_filled(
                            prev_rect,
                            CornerRadius::same(4),
                            Color32::from_rgb(rgb_edit[0], rgb_edit[1], rgb_edit[2]),
                        );
                        ui.painter().rect_stroke(
                            prev_rect,
                            CornerRadius::same(4),
                            Stroke::new(1.0_f32, rgb(t.card_border)),
                            StrokeKind::Inside,
                        );
                        let hex_resp = ui.add_sized(
                            [POPUP_INNER_W - 59.0, 22.0],
                            egui::TextEdit::singleline(&mut hex_str),
                        );
                        if hex_resp.changed() {
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(hex_buf_id, hex_str.clone());
                                d.insert_temp(hex_src_id, rgb_edit);
                            });
                            if let Some(parsed) = parse_hex_color(&hex_str) {
                                rgb_edit = parsed;
                                hsvag = HsvaGamma::from(Color32::from_rgb(
                                    parsed[0], parsed[1], parsed[2],
                                ));
                                edited = true;
                            }
                        }
                    });

                    ui.ctx().data_mut(|d| d.insert_temp(hsv_cache_id, hsvag));
                    if edited {
                        let out_color = Color32::from(Hsva::from(hsvag));
                        let out_rgb = [out_color.r(), out_color.g(), out_color.b()];
                        if out_rgb != *srgb {
                            *srgb = out_rgb;
                            button_response.mark_changed();
                        }
                    }
                });
            })
            .response;

        if !button_response.clicked()
            && (ui.input(|i| i.key_pressed(Key::Escape)) || area_response.clicked_elsewhere())
        {
            ui.memory_mut(|m| m.close_popup());
        }
    }

    button_response
}

fn parse_hex_color(input: &str) -> Option<[u8; 3]> {
    let hex = input.trim().strip_prefix('#').unwrap_or(input.trim());
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
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
pub fn clickable_warning_badge(ui: &mut egui::Ui, count: usize, color: Color32) -> egui::Response {
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

#[cfg(test)]
mod tests {
    use super::parse_hex_color;

    #[test]
    fn parse_hex_color_validates_hex_inputs() {
        assert_eq!(parse_hex_color("#00B4D8"), Some([0x00, 0xB4, 0xD8]));
        assert_eq!(parse_hex_color("  ff800a  "), Some([0xFF, 0x80, 0x0A]));
        assert_eq!(parse_hex_color("12345"), None);
        assert_eq!(parse_hex_color("#1234567"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
    }
}
