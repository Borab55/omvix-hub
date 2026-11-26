use egui::{
    ecolor::{Hsva, HsvaGamma},
    lerp, pos2, remap_clamp, vec2, Area, Color32, CornerRadius, DragValue, FontData,
    FontDefinitions, FontFamily, Frame, Key, Margin, Mesh, Order, Response, Sense, Shape, Stroke,
    StrokeKind, Style, TextWrapMode, Ui, UiKind, Vec2, Visuals,
};
use omvix_core::{CustomThemeConfig, RgbColor};
use std::fs;
use std::path::Path;

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn accent_color(t: &CustomThemeConfig) -> Color32 {
    rgb(t.accent)
}

pub fn sidebar_fill(t: &CustomThemeConfig) -> Color32 {
    rgb(t.sidebar_bg)
}

pub fn status_ok(t: &CustomThemeConfig) -> Color32 {
    rgb(t.status_ok)
}

pub fn status_warn(t: &CustomThemeConfig) -> Color32 {
    rgb(t.status_warn)
}

pub fn status_err(t: &CustomThemeConfig) -> Color32 {
    rgb(t.status_err)
}

pub fn muted_text(t: &CustomThemeConfig) -> Color32 {
    rgb(t.text_muted)
}

pub fn card_corner_radius(t: &CustomThemeConfig) -> CornerRadius {
    CornerRadius::same(t.card_radius.clamp(0, 14))
}

pub fn compact_card_corner_radius(t: &CustomThemeConfig) -> CornerRadius {
    CornerRadius::same(t.card_radius.clamp(0, 14).saturating_sub(1))
}

pub fn control_corner_radius(t: &CustomThemeConfig) -> CornerRadius {
    CornerRadius::same(t.control_radius.clamp(0, 12))
}

pub fn border_stroke_width(t: &CustomThemeConfig) -> f32 {
    let w = f32::from(t.border_width_x10.clamp(5, 25)) / 10.0;
    if t.is_high_contrast() {
        w.max(1.5)
    } else {
        w
    }
}

fn shift_color(c: [u8; 3], delta: i16) -> Color32 {
    let clamp = |v: u8| -> u8 { (v as i16 + delta).clamp(0, 255) as u8 };
    Color32::from_rgb(clamp(c[0]), clamp(c[1]), clamp(c[2]))
}

fn is_bright_color(c: Color32) -> bool {
    let luma = 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
    luma > 160.0
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
    let muted = rgb(t.text_muted);
    let ctrl_bg = rgb(t.control_bg);
    let ctrl_hover = rgb(t.control_hover_bg);
    let border_w = border_stroke_width(t);

    visuals.panel_fill = canvas;
    visuals.window_fill = canvas;
    visuals.extreme_bg_color = if t.dark_base {
        shift_color(t.control_bg, -8)
    } else {
        shift_color(t.control_bg, 8)
    };
    visuals.faint_bg_color = card;

    visuals.widgets.noninteractive.bg_fill = card;
    visuals.widgets.noninteractive.weak_bg_fill = muted;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(border_w, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.inactive.bg_fill = ctrl_bg;
    visuals.widgets.inactive.bg_stroke = Stroke::new(border_w, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.widgets.hovered.bg_fill = ctrl_hover;
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

    visuals.widgets.open.bg_fill = ctrl_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(border_w, accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, text);

    visuals.warn_fg_color = status_warn(t);
    visuals.error_fg_color = status_err(t);
    visuals.hyperlink_color = accent;

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

    let r = control_corner_radius(t);
    visuals.widgets.noninteractive.corner_radius = r;
    visuals.widgets.inactive.corner_radius = r;
    visuals.widgets.hovered.corner_radius = r;
    visuals.widgets.active.corner_radius = r;
    visuals.widgets.open.corner_radius = r;

    style.visuals = visuals;
    ctx.set_style(style);
}

pub fn card_frame(t: &CustomThemeConfig) -> egui::Frame {
    let border_w = border_stroke_width(t);
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(border_w, rgb(t.card_border)))
        .corner_radius(card_corner_radius(t))
        .inner_margin(Margin::same(14))
}

pub fn compact_card(t: &CustomThemeConfig) -> egui::Frame {
    let border_w = border_stroke_width(t);
    egui::Frame::NONE
        .fill(rgb(t.card_bg))
        .stroke(Stroke::new(border_w, rgb(t.card_border)))
        .corner_radius(compact_card_corner_radius(t))
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
    let border_w = border_stroke_width(t);
    if selected {
        if t.is_high_contrast() {
            let fg = if is_bright_color(accent) {
                Color32::BLACK
            } else {
                Color32::WHITE
            };
            (
                accent,
                Stroke::new((border_w + 0.5).max(2.0), Color32::WHITE),
                fg,
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    if t.dark_base { 54 } else { 38 },
                ),
                Stroke::new((border_w + 0.3).max(1.3), accent),
                if t.dark_base { Color32::WHITE } else { accent },
            )
        }
    } else {
        (
            rgb(t.control_bg),
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
        .corner_radius(control_corner_radius(t))
        .truncate();
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
        ui.add_sized([width, height], btn)
    })
    .inner
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileIcon {
    Feather,
    Gauge,
    Rocket,
    FanAuto,
    FanCurve,
    FanSliders,
    FanMax,
}

fn draw_tile_icon(p: &egui::Painter, c: egui::Pos2, icon: TileIcon, color: Color32) {
    let s = Stroke::new(1.45_f32, color);
    let thin = Stroke::new(1.15_f32, color);
    match icon {
        TileIcon::Feather => {
            // Quiet / silent crescent moon + soft wave
            let steps = 12usize;
            let mut arc = Vec::with_capacity(steps + 1);
            for i in 0..=steps {
                let a = lerp(
                    -0.35 * std::f32::consts::PI..=1.15 * std::f32::consts::PI,
                    i as f32 / steps as f32,
                );
                arc.push(pos2(c.x + 5.5 * a.cos(), c.y + 5.5 * a.sin()));
            }
            p.add(Shape::line(arc, s));
            p.line_segment(
                [pos2(c.x + 2.0, c.y - 5.0), pos2(c.x + 4.5, c.y - 5.0)],
                thin,
            );
            p.line_segment(
                [pos2(c.x + 3.25, c.y - 6.2), pos2(c.x + 3.25, c.y - 3.8)],
                thin,
            );
        }
        TileIcon::Gauge => {
            // Speedometer dial arc + needle
            let steps = 14usize;
            let mut arc = Vec::with_capacity(steps + 1);
            for i in 0..=steps {
                let a = lerp(std::f32::consts::PI..=0.0, i as f32 / steps as f32);
                arc.push(pos2(c.x + 6.4 * a.cos(), (c.y + 2.2) - 6.4 * a.sin()));
            }
            p.add(Shape::line(arc, s));
            p.line_segment([pos2(c.x, c.y + 2.4), pos2(c.x + 3.4, c.y - 2.6)], s);
            p.circle_filled(pos2(c.x, c.y + 2.4), 1.5, color);
        }
        TileIcon::Rocket => {
            // Sharp performance bolt / rocket nose
            let bolt = vec![
                pos2(c.x + 1.2, c.y - 6.5),
                pos2(c.x - 4.2, c.y + 0.8),
                pos2(c.x - 0.3, c.y + 0.8),
                pos2(c.x - 1.4, c.y + 6.5),
                pos2(c.x + 4.4, c.y - 0.8),
                pos2(c.x + 0.4, c.y - 0.8),
            ];
            p.add(Shape::closed_line(bolt, s));
        }
        TileIcon::FanAuto => {
            p.circle_stroke(c, 2.0, s);
            for k in 0..3 {
                let a = (k as f32) * (std::f32::consts::TAU / 3.0) - 0.5;
                let b = a + 0.55;
                p.line_segment(
                    [
                        pos2(c.x + 2.2 * a.cos(), c.y + 2.2 * a.sin()),
                        pos2(c.x + 6.3 * b.cos(), c.y + 6.3 * b.sin()),
                    ],
                    s,
                );
            }
        }
        TileIcon::FanCurve => {
            // Mini chart axes + rising curve
            p.line_segment(
                [pos2(c.x - 5.8, c.y - 5.2), pos2(c.x - 5.8, c.y + 4.8)],
                thin,
            );
            p.line_segment(
                [pos2(c.x - 5.8, c.y + 4.8), pos2(c.x + 6.0, c.y + 4.8)],
                thin,
            );
            let curve = vec![
                pos2(c.x - 4.5, c.y + 3.0),
                pos2(c.x - 1.0, c.y + 1.4),
                pos2(c.x + 2.2, c.y - 1.8),
                pos2(c.x + 5.2, c.y - 4.4),
            ];
            p.add(Shape::line(curve, s));
            p.circle_filled(pos2(c.x - 1.0, c.y + 1.4), 1.4, color);
            p.circle_filled(pos2(c.x + 5.2, c.y - 4.4), 1.4, color);
        }
        TileIcon::FanSliders => {
            p.line_segment(
                [pos2(c.x - 5.6, c.y - 2.6), pos2(c.x + 5.6, c.y - 2.6)],
                thin,
            );
            p.circle_filled(pos2(c.x + 1.8, c.y - 2.6), 1.9, color);
            p.line_segment(
                [pos2(c.x - 5.6, c.y + 2.6), pos2(c.x + 5.6, c.y + 2.6)],
                thin,
            );
            p.circle_filled(pos2(c.x - 2.0, c.y + 2.6), 1.9, color);
        }
        TileIcon::FanMax => {
            p.circle_stroke(c, 6.2, thin);
            p.circle_filled(c, 1.5, color);
            for k in 0..4 {
                let a = (k as f32) * (std::f32::consts::FRAC_PI_2) + 0.3;
                p.line_segment(
                    [
                        pos2(c.x + 1.8 * a.cos(), c.y + 1.8 * a.sin()),
                        pos2(c.x + 4.8 * a.cos(), c.y + 4.8 * a.sin()),
                    ],
                    s,
                );
            }
        }
    }
}

/// G-Helper-style compact rounded tile with centered vector icon above the label and crisp active border.
#[allow(clippy::too_many_arguments)]
pub fn mode_tile_button(
    ui: &mut Ui,
    icon: TileIcon,
    label: &str,
    selected: bool,
    active_border: Option<Color32>,
    width: f32,
    height: f32,
    t: &CustomThemeConfig,
) -> Response {
    let accent = active_border.unwrap_or_else(|| accent_color(t));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fill = if selected {
            if t.is_high_contrast() {
                accent
            } else {
                Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    if t.dark_base { 28 } else { 24 },
                )
            }
        } else if resp.hovered() {
            rgb(t.control_hover_bg)
        } else {
            rgb(t.control_bg)
        };
        let border_w = border_stroke_width(t);
        let stroke = if selected {
            Stroke::new(
                (border_w + 0.8).max(1.8),
                if t.is_high_contrast() {
                    Color32::WHITE
                } else {
                    accent
                },
            )
        } else if resp.hovered() {
            Stroke::new((border_w + 0.1).max(1.1), accent)
        } else {
            Stroke::new(border_w, rgb(t.card_border))
        };
        let fg = if selected {
            if t.is_high_contrast() {
                if is_bright_color(accent) {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                }
            } else if t.dark_base {
                Color32::WHITE
            } else {
                accent
            }
        } else {
            rgb(t.text_primary)
        };
        let icon_color = if selected {
            if t.is_high_contrast() {
                fg
            } else {
                accent
            }
        } else {
            rgb(t.text_primary)
        };

        let r = control_corner_radius(t);
        p.rect_filled(rect, r, fill);
        p.rect_stroke(rect.shrink(0.5), r, stroke, StrokeKind::Inside);

        let icon_c = pos2(
            rect.center().x,
            rect.top() + (height * 0.36).clamp(13.0, 19.0),
        );
        draw_tile_icon(&p, icon_c, icon, icon_color);

        let font_sz = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Button)
            .map(|f| f.size * 0.88)
            .unwrap_or(11.5);
        let mut rt = egui::RichText::new(label).size(font_sz).color(fg);
        if selected {
            rt = rt.strong();
        }
        let galley = egui::WidgetText::from(rt).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            (width - 8.0).max(24.0),
            egui::TextStyle::Button,
        );
        let text_y = rect.bottom() - (height * 0.25).clamp(10.0, 15.0) - galley.size().y * 0.5;
        let text_pos = pos2(rect.center().x - galley.size().x * 0.5, text_y);
        p.galley(text_pos, galley, fg);
    }

    resp
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
        .corner_radius(control_corner_radius(t))
        .wrap_mode(TextWrapMode::Extend);
    let prev_pad = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
    let resp = ui.add_enabled(enabled, btn);
    ui.spacing_mut().button_padding = prev_pad;
    resp
}

fn compute_toggle_layout(
    btn_text_size: Vec2,
    tag_text_size: Vec2,
    active: bool,
) -> (Vec2, Vec2, Vec2) {
    const BTN_PAD_X: f32 = 9.0;
    const BTN_PAD_Y: f32 = 4.0;
    const TAG_PAD_X: f32 = 6.0;
    const TAG_PAD_Y: f32 = 2.0;
    const TAG_GAP: f32 = 5.0;

    let btn_size = vec2(
        (btn_text_size.x + BTN_PAD_X * 2.0).ceil(),
        (btn_text_size.y + BTN_PAD_Y * 2.0).ceil().max(24.0),
    );
    let tag_size = vec2(
        (tag_text_size.x + TAG_PAD_X * 2.0).ceil(),
        (tag_text_size.y + TAG_PAD_Y * 2.0).ceil().max(18.0),
    );
    let total_w = if active {
        btn_size.x + TAG_GAP + tag_size.x
    } else {
        btn_size.x
    };
    let total_h = if active {
        btn_size.y.max(tag_size.y)
    } else {
        btn_size.y
    };
    (btn_size, tag_size, vec2(total_w, total_h))
}

fn render_toggle_button(
    ui: &mut Ui,
    label: &str,
    active: &mut bool,
    active_tag: &str,
    header_emphasis: bool,
    t: &CustomThemeConfig,
) -> Response {
    let was_active = *active;
    let accent = accent_color(t);
    let muted = muted_text(t);
    let ok = status_ok(t);
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

    let label_color = if was_active {
        if t.is_high_contrast() {
            if is_bright_color(accent) {
                Color32::BLACK
            } else {
                Color32::WHITE
            }
        } else if t.dark_base {
            Color32::WHITE
        } else {
            accent
        }
    } else {
        muted
    };

    // Keep text and button width invariant across ON/OFF and allocate one atomic rect
    // directly on `ui` so `ui.horizontal_wrapped` wraps to the next line cleanly.
    let mut rt = egui::RichText::new(label)
        .size(text_size)
        .color(label_color);
    if header_emphasis {
        rt = rt.strong();
    }
    let btn_widget = egui::WidgetText::from(rt);
    let mut btn_galley = btn_widget.clone().into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let tag_galley = egui::WidgetText::from(
        egui::RichText::new(active_tag)
            .size(10.5)
            .color(ok)
            .strong(),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );

    let tag_reserve = (tag_galley.size().x + 12.0).ceil() + 5.0;
    let max_btn_text_w = (ui.max_rect().width() - tag_reserve - 18.0).max(80.0);
    if btn_galley.size().x > max_btn_text_w {
        btn_galley = btn_widget.into_galley(
            ui,
            Some(TextWrapMode::Wrap),
            max_btn_text_w,
            egui::TextStyle::Button,
        );
    }

    let (btn_size, tag_size, total_size) =
        compute_toggle_layout(btn_galley.size(), tag_galley.size(), was_active);
    let (outer_rect, alloc_resp) = ui.allocate_exact_size(total_size, Sense::hover());
    let btn_rect = egui::Rect::from_min_size(
        pos2(outer_rect.left(), outer_rect.center().y - btn_size.y * 0.5),
        btn_size,
    );
    let mut resp = ui.interact(btn_rect, alloc_resp.id.with("toggle_btn"), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *active = !*active;
        resp.mark_changed();
    }

    if ui.is_rect_visible(outer_rect) {
        let border_w = border_stroke_width(t);
        let (fill, stroke) = if was_active {
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
            (bg, Stroke::new((border_w + 0.3).max(1.3), accent))
        } else if resp.hovered() {
            (
                rgb(t.control_hover_bg),
                Stroke::new((border_w + 0.1).max(1.1), accent),
            )
        } else {
            (rgb(t.control_bg), Stroke::new(border_w, rgb(t.card_border)))
        };

        let p = ui.painter_at(outer_rect);
        let r = control_corner_radius(t);
        p.rect_filled(btn_rect, r, fill);
        p.rect_stroke(btn_rect, r, stroke, StrokeKind::Inside);

        let btn_text_pos = pos2(
            btn_rect.left() + (btn_rect.width() - btn_galley.size().x) * 0.5,
            btn_rect.center().y - btn_galley.size().y * 0.5,
        );
        p.galley(btn_text_pos, btn_galley, label_color);

        if was_active {
            let tag_rect = egui::Rect::from_min_size(
                pos2(
                    btn_rect.right() + 5.0,
                    outer_rect.center().y - tag_size.y * 0.5,
                ),
                tag_size,
            );
            let tag_bg = Color32::from_rgba_unmultiplied(ok.r(), ok.g(), ok.b(), 26);
            let tag_cr = CornerRadius::same(t.control_radius.clamp(2, 10));
            p.rect_filled(tag_rect, tag_cr, tag_bg);
            p.rect_stroke(
                tag_rect,
                tag_cr,
                Stroke::new(0.8_f32, ok),
                StrokeKind::Inside,
            );
            let tag_pos = pos2(
                tag_rect.left() + (tag_rect.width() - tag_galley.size().x) * 0.5,
                tag_rect.center().y - tag_galley.size().y * 0.5,
            );
            p.galley(tag_pos, tag_galley, ok);
        }
    }

    resp
}

/// Illuminated text-button toggle for feature options (lights up with external `active_tag` pill when active,
/// dims subtly with tag hidden when off without changing button width).
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

/// Compact rounded-square sliders/tune button that opens the corresponding side-drawer detail view.
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
                if t.dark_base { 58 } else { 40 },
            )
        } else if resp.hovered() {
            rgb(t.control_hover_bg)
        } else {
            rgb(t.control_bg)
        };
        let stroke_color = if active || resp.hovered() {
            accent
        } else {
            rgb(t.card_border)
        };
        let bar_fg = if active || resp.hovered() {
            if t.dark_base {
                Color32::WHITE
            } else {
                rgb(t.text_primary)
            }
        } else {
            muted_text(t)
        };
        let knob_c = if active || resp.hovered() {
            accent
        } else {
            rgb(t.text_primary)
        };

        let r = control_corner_radius(t);
        let border_w = border_stroke_width(t);
        p.rect_filled(rect, r, fill);
        p.rect_stroke(
            rect.shrink(0.5),
            r,
            Stroke::new(
                if active || resp.hovered() {
                    (border_w + 0.3).max(1.3)
                } else {
                    border_w
                },
                stroke_color,
            ),
            StrokeKind::Inside,
        );

        // Clean 3-rail vector "tune / detailed settings" sliders icon
        let c = rect.center();
        let x0 = c.x - 5.2;
        let x1 = c.x + 5.2;
        let rails = [
            (c.y - 4.0, c.x - 1.8),
            (c.y, c.x + 2.2),
            (c.y + 4.0, c.x - 0.6),
        ];
        let bar_stroke = Stroke::new(1.3_f32, bar_fg);
        for (ry, kx) in rails {
            p.line_segment([pos2(x0, ry), pos2(x1, ry)], bar_stroke);
            p.circle_filled(pos2(kx, ry), 1.9, knob_c);
        }
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
    let size = Vec2::splat(26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let inner = rect.shrink(2.2);
        let inner_r_val = t.control_radius.clamp(0, 12).saturating_sub(1);
        let inner_r = CornerRadius::same(inner_r_val);
        let solid = zones.iter().all(|z| *z == zones[0]);
        if solid {
            let c = Color32::from_rgb(zones[0].r, zones[0].g, zones[0].b);
            p.rect_filled(inner, inner_r, c);
        } else {
            let strip_w = inner.width() / 4.0;
            for (idx, z) in zones.iter().enumerate() {
                let x0 = inner.left() + idx as f32 * strip_w;
                let x1 = if idx == 3 {
                    inner.right()
                } else {
                    x0 + strip_w
                };
                let strip =
                    egui::Rect::from_min_max(pos2(x0, inner.top()), pos2(x1, inner.bottom()));
                let cr = match idx {
                    0 => CornerRadius {
                        nw: inner_r_val,
                        sw: inner_r_val,
                        ne: 0,
                        se: 0,
                    },
                    3 => CornerRadius {
                        nw: 0,
                        sw: 0,
                        ne: inner_r_val,
                        se: inner_r_val,
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
            inner_r,
            Stroke::new(1.0_f32, inner_ring),
            StrokeKind::Inside,
        );

        let border_w = border_stroke_width(t);
        let outer_stroke = if selected {
            Stroke::new((border_w + 1.0).max(2.0), accent)
        } else if resp.hovered() {
            Stroke::new((border_w + 0.4).max(1.4), accent)
        } else {
            Stroke::new(border_w, rgb(t.card_border))
        };
        let outer_r = control_corner_radius(t);
        p.rect_stroke(rect.shrink(0.5), outer_r, outer_stroke, StrokeKind::Inside);
    }

    resp.on_hover_text(tooltip)
}

/// Unified keyboard lighting profile button with a top color cap directly above the profile label.
pub fn lighting_profile_button(
    ui: &mut Ui,
    label: &str,
    zones: &[RgbColor; 4],
    selected: bool,
    four_zone: bool,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let text_c = rgb(t.text_primary);
    let btn_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| f.size * 0.92)
        .unwrap_or(12.0);
    let mut rt = egui::RichText::new(label).size(btn_size).color(text_c);
    if selected {
        rt = rt.strong();
    }
    let galley = egui::WidgetText::from(rt).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );

    let cap_h = 5.0_f32;
    let width = (galley.size().x + 18.0).ceil().max(42.0);
    let height = 26.0_f32.max((galley.size().y + 12.0).ceil());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fill = if selected {
            let w_accent = if t.is_high_contrast() { 0.28 } else { 0.20 };
            let mix = |bg: u8, fg: u8| -> u8 {
                (f32::from(bg) * (1.0 - w_accent) + f32::from(fg) * w_accent)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            Color32::from_rgb(
                mix(t.control_bg[0], t.accent[0]),
                mix(t.control_bg[1], t.accent[1]),
                mix(t.control_bg[2], t.accent[2]),
            )
        } else if resp.hovered() {
            rgb(t.control_hover_bg)
        } else {
            rgb(t.control_bg)
        };
        let outer_r = control_corner_radius(t);
        let cap_r_val = t.control_radius.clamp(0, 12).saturating_sub(1);
        p.rect_filled(rect, outer_r, fill);

        let cap_rect = egui::Rect::from_min_max(
            pos2(rect.left() + 1.5, rect.top() + 1.5),
            pos2(rect.right() - 1.5, rect.top() + 1.5 + cap_h),
        );
        let cap_radius = CornerRadius {
            nw: cap_r_val,
            ne: cap_r_val,
            sw: 0,
            se: 0,
        };
        let solid = !four_zone || zones.iter().all(|z| *z == zones[0]);
        if solid {
            let c = Color32::from_rgb(zones[0].r, zones[0].g, zones[0].b);
            p.rect_filled(cap_rect, cap_radius, c);
        } else {
            let strip_w = cap_rect.width() / 4.0;
            for (idx, z) in zones.iter().enumerate() {
                let x0 = cap_rect.left() + idx as f32 * strip_w;
                let x1 = if idx == 3 {
                    cap_rect.right()
                } else {
                    x0 + strip_w
                };
                let seg =
                    egui::Rect::from_min_max(pos2(x0, cap_rect.top()), pos2(x1, cap_rect.bottom()));
                let cr = match idx {
                    0 => CornerRadius {
                        nw: cap_r_val,
                        ne: 0,
                        sw: 0,
                        se: 0,
                    },
                    3 => CornerRadius {
                        nw: 0,
                        ne: cap_r_val,
                        sw: 0,
                        se: 0,
                    },
                    _ => CornerRadius::ZERO,
                };
                p.rect_filled(seg, cr, Color32::from_rgb(z.r, z.g, z.b));
            }
        }

        let sep_alpha = if t.dark_base { 90 } else { 36 };
        p.line_segment(
            [
                pos2(cap_rect.left(), cap_rect.bottom()),
                pos2(cap_rect.right(), cap_rect.bottom()),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 0, 0, sep_alpha)),
        );

        let border_w = border_stroke_width(t);
        let outer_stroke = if selected {
            Stroke::new((border_w + 0.8).max(1.8), accent)
        } else if resp.hovered() {
            Stroke::new((border_w + 0.3).max(1.3), accent)
        } else {
            Stroke::new(border_w, rgb(t.card_border))
        };
        p.rect_stroke(rect.shrink(0.5), outer_r, outer_stroke, StrokeKind::Inside);

        let text_center_y = ((cap_rect.bottom() + 0.5) + (rect.bottom() - 2.5)) * 0.5;
        let text_pos = pos2(
            rect.center().x - galley.size().x * 0.5,
            text_center_y - galley.size().y * 0.5,
        );
        p.galley(text_pos, galley, text_c);
    }

    resp.on_hover_text(label)
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

#[allow(clippy::too_many_arguments)]
fn paint_pill_badge(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    font_size: f32,
    pad: Vec2,
    radius: u8,
    stroke_w: f32,
    bg_alpha: u8,
) -> egui::Response {
    let galley = egui::WidgetText::from(
        egui::RichText::new(text)
            .size(font_size)
            .color(color)
            .strong(),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = vec2(
        (galley.size().x + pad.x * 2.0).ceil(),
        (galley.size().y + pad.y * 2.0).ceil(),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), bg_alpha);
        p.rect_filled(rect, CornerRadius::same(radius), bg);
        p.rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(stroke_w, color),
            StrokeKind::Inside,
        );
        let text_pos = pos2(
            rect.left() + (rect.width() - galley.size().x) * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        p.galley(text_pos, galley, color);
    }
    resp
}

pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    paint_pill_badge(ui, text, color, 12.0, vec2(10.0, 4.0), 12, 1.0, 32)
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

/// Compact illuminated pill toggle used for permanent inline cockpit switches (e.g., Auto AC/DC).
pub fn compact_toggle_pill(
    ui: &mut Ui,
    label: &str,
    active: bool,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let muted = muted_text(t);
    let border_w = border_stroke_width(t);
    let (fill, stroke, text_c) = if active {
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
        (bg, Stroke::new((border_w + 0.1).max(1.1), accent), fg)
    } else {
        (
            rgb(t.control_bg),
            Stroke::new((border_w * 0.9).max(0.5), rgb(t.card_border)),
            muted,
        )
    };
    let mut rt = egui::RichText::new(label).size(10.5).color(text_c);
    if active {
        rt = rt.strong();
    }
    let prev_pad = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = vec2(7.0, 2.0);
    let resp = ui.add(
        egui::Button::new(rt)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(
                t.control_radius.clamp(0, 12).saturating_add(2).min(12),
            ))
            .wrap_mode(TextWrapMode::Extend),
    );
    ui.spacing_mut().button_padding = prev_pad;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Borderless text link button for the compact cockpit footer (no outer box/border).
pub fn footer_text_button(
    ui: &mut Ui,
    label: &str,
    font_size: f32,
    base_color: Color32,
    hover_color: Color32,
) -> Response {
    let galley =
        egui::WidgetText::from(egui::RichText::new(label).size(font_size).color(base_color))
            .into_galley(
                ui,
                Some(TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );

    let pad = vec2(4.0, 2.0);
    let size = vec2(
        (galley.size().x + pad.x * 2.0).ceil(),
        (galley.size().y + pad.y * 2.0).ceil().max(20.0),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fg = if resp.hovered() {
            hover_color
        } else {
            base_color
        };
        let paint_galley = if resp.hovered() {
            egui::WidgetText::from(egui::RichText::new(label).size(font_size).color(fg))
                .into_galley(
                    ui,
                    Some(TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                )
        } else {
            galley
        };
        let text_pos = pos2(
            rect.left() + pad.x,
            rect.center().y - paint_galley.size().y * 0.5,
        );
        p.galley(text_pos, paint_galley, fg);
    }

    resp
}

/// Borderless footer undo button with a custom vector curved undo arrow + label (no outer box/border).
pub fn undo_text_button(
    ui: &mut Ui,
    label: &str,
    font_size: f32,
    base_color: Color32,
    t: &CustomThemeConfig,
) -> Response {
    let accent = accent_color(t);
    let probe_galley =
        egui::WidgetText::from(egui::RichText::new(label).size(font_size).color(base_color))
            .into_galley(
                ui,
                Some(TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );

    let pad_x = 4.0_f32;
    let icon_w = 12.0_f32;
    let gap = 5.0_f32;
    let width = (pad_x * 2.0 + icon_w + gap + probe_galley.size().x).ceil();
    let height = (probe_galley.size().y + 4.0).ceil().max(20.0);

    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let fg = if resp.hovered() { accent } else { base_color };
        let galley = if resp.hovered() {
            egui::WidgetText::from(egui::RichText::new(label).size(font_size).color(fg))
                .into_galley(
                    ui,
                    Some(TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                )
        } else {
            probe_galley
        };

        let cx = rect.left() + pad_x + icon_w * 0.5 + 0.5;
        let cy = rect.center().y + 0.5;
        let r = 4.3_f32;
        let start = 0.30_f32 * std::f32::consts::PI;
        let end = -0.86_f32 * std::f32::consts::PI;
        let steps = 10usize;
        let mut pts = Vec::with_capacity(steps + 1);
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            let a = lerp(start..=end, f);
            pts.push(pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
        let icon_stroke = Stroke::new(1.4_f32, fg);
        p.add(Shape::line(pts.clone(), icon_stroke));
        if let Some(&tip) = pts.last() {
            p.line_segment([tip, pos2(tip.x + 3.2, tip.y - 1.4)], icon_stroke);
            p.line_segment([tip, pos2(tip.x - 0.9, tip.y - 3.4)], icon_stroke);
        }

        let text_pos = pos2(
            rect.left() + pad_x + icon_w + gap,
            rect.center().y - galley.size().y * 0.5,
        );
        p.galley(text_pos, galley, fg);
    }

    resp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedChipAction {
    None,
    Apply,
    Delete,
}

/// Saved custom theme chip with fixed width and inline circular `×` delete affordance revealed on hover.
pub fn saved_theme_chip(
    ui: &mut Ui,
    name: &str,
    accent_dot: Color32,
    active: bool,
    delete_tooltip: &str,
    t: &CustomThemeConfig,
) -> SavedChipAction {
    let accent = accent_color(t);
    let err_c = status_err(t);
    let (_, _, text_c) = button_palette(active, t);
    let btn_size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| f.size * 0.95)
        .unwrap_or(12.5);
    let mut rt = egui::RichText::new(name).size(btn_size).color(text_c);
    if active {
        rt = rt.strong();
    }
    let galley = egui::WidgetText::from(rt).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );

    let pad_l = 9.0_f32;
    let pad_r = 6.0_f32;
    let dot_d = 9.0_f32;
    let gap = 6.0_f32;
    let close_d = 16.0_f32;
    let width = (pad_l + dot_d + gap + galley.size().x + gap + close_d + pad_r).ceil();
    let height = 26.0_f32;

    let (pill_rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let close_center = pos2(
        pill_rect.right() - pad_r - close_d * 0.5,
        pill_rect.center().y,
    );
    let close_rect = egui::Rect::from_center_size(close_center, Vec2::splat(close_d));
    let apply_rect = egui::Rect::from_min_max(
        pill_rect.min,
        pos2(close_rect.left() - 2.0, pill_rect.bottom()),
    );

    let apply_resp = ui.interact(
        apply_rect,
        ui.id().with(("theme_chip_apply", name)),
        Sense::click(),
    );
    let del_resp = ui
        .interact(
            close_rect,
            ui.id().with(("theme_chip_del", name)),
            Sense::click(),
        )
        .on_hover_text(delete_tooltip);

    let chip_hovered = ui.rect_contains_pointer(pill_rect);
    if apply_resp.hovered() || del_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(pill_rect) {
        let p = ui.painter_at(pill_rect);
        let (base_fill, base_stroke, _) = button_palette(active, t);
        let fill = if !active && chip_hovered {
            rgb(t.control_hover_bg)
        } else {
            base_fill
        };
        let stroke = if active || chip_hovered {
            Stroke::new(1.3_f32, accent)
        } else {
            base_stroke
        };

        let cr = control_corner_radius(t);
        p.rect_filled(pill_rect, cr, fill);
        p.rect_stroke(pill_rect.shrink(0.5), cr, stroke, StrokeKind::Inside);

        let dot_center = pos2(pill_rect.left() + pad_l + dot_d * 0.5, pill_rect.center().y);
        p.circle_filled(dot_center, dot_d * 0.5, accent_dot);

        let text_pos = pos2(
            pill_rect.left() + pad_l + dot_d + gap,
            pill_rect.center().y - galley.size().y * 0.5,
        );
        p.galley(text_pos, galley, text_c);

        if chip_hovered || del_resp.hovered() {
            let x_bg = if del_resp.hovered() {
                Color32::from_rgba_unmultiplied(err_c.r(), err_c.g(), err_c.b(), 64)
            } else {
                Color32::from_rgba_unmultiplied(text_c.r(), text_c.g(), text_c.b(), 24)
            };
            let x_fg = if del_resp.hovered() {
                err_c
            } else {
                muted_text(t)
            };
            p.circle_filled(close_center, close_d * 0.48, x_bg);
            let r = 3.1_f32;
            let xs = Stroke::new(1.4_f32, x_fg);
            p.line_segment(
                [
                    pos2(close_center.x - r, close_center.y - r),
                    pos2(close_center.x + r, close_center.y + r),
                ],
                xs,
            );
            p.line_segment(
                [
                    pos2(close_center.x + r, close_center.y - r),
                    pos2(close_center.x - r, close_center.y + r),
                ],
                xs,
            );
        }
    }

    if del_resp.clicked() {
        SavedChipAction::Delete
    } else if apply_resp.clicked() {
        SavedChipAction::Apply
    } else {
        SavedChipAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_toggle_layout, parse_hex_color};
    use egui::vec2;

    #[test]
    fn parse_hex_color_validates_hex_inputs() {
        assert_eq!(parse_hex_color("#00B4D8"), Some([0x00, 0xB4, 0xD8]));
        assert_eq!(parse_hex_color("  ff800a  "), Some([0xFF, 0x80, 0x0A]));
        assert_eq!(parse_hex_color("12345"), None);
        assert_eq!(parse_hex_color("#1234567"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
    }

    #[test]
    fn toggle_layout_keeps_button_width_constant_and_accounts_for_external_tag() {
        let label_sz = vec2(120.4, 15.2);
        let tag_sz = vec2(28.2, 13.0);
        let (btn_off, _tag_off, total_off) = compute_toggle_layout(label_sz, tag_sz, false);
        let (btn_on, tag_on, total_on) = compute_toggle_layout(label_sz, tag_sz, true);

        assert_eq!(btn_off, btn_on);
        assert_eq!(total_off, btn_off);
        assert_eq!(total_on.x, btn_on.x + 5.0 + tag_on.x);
        assert!(total_on.y >= btn_on.y.max(tag_on.y));
    }
}
