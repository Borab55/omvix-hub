use crate::theme::{STATUS_WARN, accent_color, rgb};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2};
use omvix_core::{BatteryHealthRecord, CustomThemeConfig, FanCurvePoint};

pub struct ChartSeries<'a> {
    pub name: &'a str,
    pub color: Color32,
    pub unit: &'a str,
    pub points: &'a [(f64, f32)],
    pub fill_alpha: u8,
}

#[derive(Default, Clone, Copy)]
pub struct FanCurveEditResult {
    pub changed: bool,
    pub committed: bool,
}

fn chart_bg_color(t: &CustomThemeConfig) -> Color32 {
    if t.dark_base {
        Color32::from_rgb(
            t.canvas_bg[0].saturating_sub(4),
            t.canvas_bg[1].saturating_sub(4),
            t.canvas_bg[2].saturating_sub(4),
        )
    } else {
        Color32::from_rgb(248, 250, 252)
    }
}

fn muted_text_color(t: &CustomThemeConfig) -> Color32 {
    if t.dark_base {
        Color32::from_rgb(156, 160, 170)
    } else {
        Color32::from_rgb(100, 110, 125)
    }
}

fn subtle_grid_color(t: &CustomThemeConfig) -> Color32 {
    if t.dark_base {
        Color32::from_rgba_unmultiplied(255, 255, 255, 18)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 18)
    }
}

/// High-DPI native `egui::Painter` multi-series area chart that NEVER intercepts mouse-wheel scrolling.
#[allow(clippy::too_many_arguments)]
pub fn telemetry_area_chart(
    ui: &mut Ui,
    _id_salt: &str,
    height: f32,
    y_min: f32,
    y_max: f32,
    unit: &str,
    series: &[ChartSeries<'_>],
    t: &CustomThemeConfig,
) {
    let total_w = ui.available_width().max(240.0);
    let legend_h = 22.0;

    // Compact legend header with live values
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(legend_h);
        ui.spacing_mut().item_spacing.x = 12.0;
        for s in series {
            if s.points.is_empty() {
                continue;
            }
            let latest = s
                .points
                .last()
                .map(|(_, y)| {
                    if *y >= 100.0 || s.unit == "°C" || s.unit == "%" {
                        format!("{y:.0}{}", s.unit)
                    } else {
                        format!("{y:.1}{}", s.unit)
                    }
                })
                .unwrap_or_else(|| "—".into());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot_rect.center(), 4.0, s.color);
                ui.label(egui::RichText::new(s.name).size(11.5).color(muted_text_color(t)));
                ui.label(egui::RichText::new(latest).size(12.0).strong().color(s.color));
            });
        }
    });

    let (outer_rect, response) =
        ui.allocate_exact_size(Vec2::new(total_w, height), egui::Sense::hover());
    let painter = ui.painter_at(outer_rect);

    let bg = chart_bg_color(t);
    let border = rgb(t.card_border);
    let grid = subtle_grid_color(t);
    let axis_text = muted_text_color(t);

    painter.rect_filled(outer_rect, CornerRadius::same(8), bg);
    painter.rect_stroke(
        outer_rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );

    let left_pad = 40.0;
    let right_pad = 12.0;
    let top_pad = 10.0;
    let bottom_pad = 20.0;
    let plot_rect = Rect::from_min_max(
        Pos2::new(outer_rect.left() + left_pad, outer_rect.top() + top_pad),
        Pos2::new(
            (outer_rect.right() - right_pad).max(outer_rect.left() + left_pad + 40.0),
            (outer_rect.bottom() - bottom_pad).max(outer_rect.top() + top_pad + 40.0),
        ),
    );

    // Determine X and Y ranges dynamically (keeping y_min..y_max floor)
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut eff_y_max = y_max;
    for s in series {
        for &(x, y) in s.points {
            if x.is_finite() {
                x_min = x_min.min(x);
                x_max = x_max.max(x);
            }
            if y.is_finite() {
                eff_y_max = eff_y_max.max(y * 1.12);
            }
        }
    }
    if !x_min.is_finite() || !x_max.is_finite() || (x_max - x_min) < 1.0 {
        x_min = 0.0;
        x_max = 10.0;
    }
    let y_span = (eff_y_max - y_min).max(1.0);
    let x_span = (x_max - x_min).max(1e-3);

    let map_pt = |x: f64, y: f32| -> Pos2 {
        let nx = ((x - x_min) / x_span).clamp(0.0, 1.0) as f32;
        let ny = ((y - y_min) / y_span).clamp(0.0, 1.0);
        Pos2::new(
            plot_rect.left() + nx * plot_rect.width(),
            plot_rect.bottom() - ny * plot_rect.height(),
        )
    };

    // Horizontal grid lines & Y labels
    let steps = 4usize;
    for i in 0..=steps {
        let frac = i as f32 / steps as f32;
        let val = y_min + frac * y_span;
        let py = plot_rect.bottom() - frac * plot_rect.height();
        painter.line_segment(
            [Pos2::new(plot_rect.left(), py), Pos2::new(plot_rect.right(), py)],
            Stroke::new(1.0_f32, grid),
        );
        let label = if i == steps {
            format!("{val:.0}{unit}")
        } else {
            format!("{val:.0}")
        };
        painter.text(
            Pos2::new(plot_rect.left() - 6.0, py),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(10.0),
            axis_text,
        );
    }

    // X span labels (relative age)
    let span_secs = (x_max - x_min).round().max(1.0) as i64;
    for (frac, lbl) in [
        (0.0_f32, format!("-{span_secs}s")),
        (0.5_f32, format!("-{}s", span_secs / 2)),
        (1.0_f32, "0s".to_string()),
    ] {
        let px = plot_rect.left() + frac * plot_rect.width();
        painter.line_segment(
            [Pos2::new(px, plot_rect.top()), Pos2::new(px, plot_rect.bottom())],
            Stroke::new(1.0_f32, grid),
        );
        let anchor = if frac <= 0.01 {
            Align2::LEFT_TOP
        } else if frac >= 0.99 {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            Pos2::new(px, plot_rect.bottom() + 4.0),
            anchor,
            lbl,
            FontId::proportional(10.0),
            axis_text,
        );
    }

    // Draw series fills + polylines
    let base_y = plot_rect.bottom();
    for s in series {
        if s.points.is_empty() {
            continue;
        }
        let screen_pts: Vec<Pos2> = s.points.iter().map(|&(x, y)| map_pt(x, y)).collect();
        if screen_pts.len() >= 2 {
            if s.fill_alpha > 0 {
                let fill = Color32::from_rgba_unmultiplied(
                    s.color.r(),
                    s.color.g(),
                    s.color.b(),
                    s.fill_alpha,
                );
                for win in screen_pts.windows(2) {
                    let p1 = win[0];
                    let p2 = win[1];
                    if p2.x - p1.x <= 0.05 || (p1.y >= base_y - 0.5 && p2.y >= base_y - 0.5) {
                        continue;
                    }
                    painter.add(Shape::convex_polygon(
                        vec![p1, p2, Pos2::new(p2.x, base_y), Pos2::new(p1.x, base_y)],
                        fill,
                        Stroke::NONE,
                    ));
                }
            }
            painter.add(Shape::line(screen_pts, Stroke::new(2.0_f32, s.color)));
        } else if let Some(&p) = screen_pts.first() {
            painter.circle_filled(p, 3.0, s.color);
        }
    }

    // Hover crosshair & multi-series tooltip
    if let Some(hover) = response.hover_pos() {
        if plot_rect.contains(hover) {
            let norm_x = ((hover.x - plot_rect.left()) / plot_rect.width().max(1.0)).clamp(0.0, 1.0);
            let hover_x = x_min + (norm_x as f64) * x_span;
            let cursor_x = plot_rect.left() + norm_x * plot_rect.width();

            painter.line_segment(
                [
                    Pos2::new(cursor_x, plot_rect.top()),
                    Pos2::new(cursor_x, plot_rect.bottom()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(220, 225, 235, 110)),
            );

            let mut lines = Vec::new();
            for s in series {
                if let Some(&(sx, sy)) = s.points.iter().min_by(|a, b| {
                    (a.0 - hover_x)
                        .abs()
                        .total_cmp(&(b.0 - hover_x).abs())
                }) {
                    let pt = map_pt(sx, sy);
                    painter.circle_filled(pt, 4.0, s.color);
                    painter.circle_stroke(pt, 4.0, Stroke::new(1.2_f32, bg));
                    lines.push(format!("{}: {:.1}{}", s.name, sy, s.unit));
                }
            }

            if !lines.is_empty() {
                let text = lines.join("  |  ");
                let text_pos = if cursor_x > plot_rect.center().x {
                    Pos2::new(plot_rect.left() + 8.0, plot_rect.top() + 8.0)
                } else {
                    Pos2::new(plot_rect.right() - 8.0, plot_rect.top() + 8.0)
                };
                let anchor = if cursor_x > plot_rect.center().x {
                    Align2::LEFT_TOP
                } else {
                    Align2::RIGHT_TOP
                };
                let galley = painter.layout_no_wrap(
                    text,
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let pill_rect = match anchor {
                    Align2::LEFT_TOP => {
                        Rect::from_min_size(text_pos - Vec2::new(4.0, 2.0), galley.size() + Vec2::new(8.0, 4.0))
                    }
                    _ => Rect::from_min_size(
                        Pos2::new(text_pos.x - galley.size().x - 4.0, text_pos.y - 2.0),
                        galley.size() + Vec2::new(8.0, 4.0),
                    ),
                };
                painter.rect_filled(
                    pill_rect,
                    CornerRadius::same(5),
                    Color32::from_rgba_unmultiplied(15, 17, 22, 220),
                );
                painter.galley(pill_rect.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
            }
        }
    }
}

/// Interactive 8-point drag-and-drop fan curve editor rendered cleanly with `egui::Painter`.
pub fn interactive_fan_curve(
    ui: &mut Ui,
    points: &mut [FanCurvePoint],
    live_temp_c: Option<f32>,
    max_fan_rpm: u32,
    t: &CustomThemeConfig,
) -> FanCurveEditResult {
    let mut res = FanCurveEditResult::default();
    let total_w = ui.available_width().max(260.0);
    let height = 225.0;

    let (outer_rect, response) =
        ui.allocate_exact_size(Vec2::new(total_w, height), egui::Sense::click_and_drag());
    let painter = ui.painter_at(outer_rect);

    let accent = accent_color(t);
    let bg = chart_bg_color(t);
    let border = rgb(t.card_border);
    let grid = subtle_grid_color(t);
    let axis_text = muted_text_color(t);

    painter.rect_filled(outer_rect, CornerRadius::same(9), bg);
    painter.rect_stroke(
        outer_rect,
        CornerRadius::same(9),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );

    let left_pad = 40.0;
    let right_pad = 14.0;
    let top_pad = 14.0;
    let bottom_pad = 24.0;
    let plot_rect = Rect::from_min_max(
        Pos2::new(outer_rect.left() + left_pad, outer_rect.top() + top_pad),
        Pos2::new(
            (outer_rect.right() - right_pad).max(outer_rect.left() + left_pad + 60.0),
            (outer_rect.bottom() - bottom_pad).max(outer_rect.top() + top_pad + 60.0),
        ),
    );

    const MIN_TEMP: f32 = 30.0;
    const MAX_TEMP: f32 = 100.0;
    const TEMP_SPAN: f32 = MAX_TEMP - MIN_TEMP;

    let temp_to_x = |temp: f32| -> f32 {
        let nx = ((temp - MIN_TEMP) / TEMP_SPAN).clamp(0.0, 1.0);
        plot_rect.left() + nx * plot_rect.width()
    };
    let pct_to_y = |pct: f32| -> f32 {
        let ny = (pct / 100.0).clamp(0.0, 1.0);
        plot_rect.bottom() - ny * plot_rect.height()
    };
    let pos_to_temp_pct = |pos: Pos2| -> (f32, f32) {
        let nx = ((pos.x - plot_rect.left()) / plot_rect.width().max(1.0)).clamp(0.0, 1.0);
        let ny = ((plot_rect.bottom() - pos.y) / plot_rect.height().max(1.0)).clamp(0.0, 1.0);
        (MIN_TEMP + nx * TEMP_SPAN, ny * 100.0)
    };

    // Highlight R2 high-temperature fan safety floor (>= 80°C requires >= 50%)
    let safe_x = temp_to_x(80.0);
    let safe_y = pct_to_y(50.0);
    let safety_floor_rect = Rect::from_min_max(
        Pos2::new(safe_x, safe_y),
        Pos2::new(plot_rect.right(), plot_rect.bottom()),
    );
    painter.rect_filled(
        safety_floor_rect,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(239, 68, 68, 22),
    );
    painter.line_segment(
        [Pos2::new(safe_x, safe_y), Pos2::new(plot_rect.right(), safe_y)],
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(239, 68, 68, 120)),
    );
    painter.text(
        Pos2::new(plot_rect.right() - 4.0, safe_y - 3.0),
        Align2::RIGHT_BOTTOM,
        "Min 50% @ >= 80°C",
        FontId::proportional(9.5),
        Color32::from_rgba_unmultiplied(239, 68, 68, 180),
    );

    // Highlight sub-1800 RPM quiet zone (0 RPM allowed only when cool; auto-guarded under load)
    let floor_top_y = pct_to_y(31.0);
    let floor_rect = Rect::from_min_max(
        Pos2::new(plot_rect.left(), floor_top_y),
        Pos2::new(plot_rect.right(), plot_rect.bottom()),
    );
    painter.rect_filled(
        floor_rect,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(245, 158, 11, 18),
    );
    painter.text(
        Pos2::new(plot_rect.left() + 6.0, plot_rect.bottom() - 4.0),
        Align2::LEFT_BOTTOM,
        "<1800 RPM (0 RPM allowed when cool; auto-guarded)",
        FontId::proportional(10.0),
        Color32::from_rgba_unmultiplied(245, 158, 11, 175),
    );

    // Y grid (0..100%)
    for pct in [0, 20, 40, 60, 80, 100] {
        let py = pct_to_y(pct as f32);
        painter.line_segment(
            [Pos2::new(plot_rect.left(), py), Pos2::new(plot_rect.right(), py)],
            Stroke::new(1.0_f32, grid),
        );
        painter.text(
            Pos2::new(plot_rect.left() - 6.0, py),
            Align2::RIGHT_CENTER,
            format!("{pct}%"),
            FontId::proportional(10.0),
            axis_text,
        );
    }

    // X grid (30..100°C)
    for temp in [30, 40, 50, 60, 70, 80, 90, 100] {
        let px = temp_to_x(temp as f32);
        painter.line_segment(
            [Pos2::new(px, plot_rect.top()), Pos2::new(px, plot_rect.bottom())],
            Stroke::new(1.0_f32, grid),
        );
        painter.text(
            Pos2::new(px, plot_rect.bottom() + 5.0),
            Align2::CENTER_TOP,
            format!("{temp}°"),
            FontId::proportional(10.0),
            axis_text,
        );
    }

    // Handle drag selection & point updates via context memory
    let drag_id = egui::Id::new("omvix_fan_curve_drag_idx");
    let mut active_idx: Option<usize> = ui.ctx().data_mut(|d| d.get_temp(drag_id)).unwrap_or(None);

    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let mut best: Option<(usize, f32)> = None;
            for (i, pt) in points.iter().enumerate() {
                let p_screen = Pos2::new(temp_to_x(pt.temp_c as f32), pct_to_y(pt.speed_pct as f32));
                let dist = pos.distance(p_screen);
                if best.is_none_or(|(_, bd)| dist < bd) {
                    best = Some((i, dist));
                }
            }
            if let Some((i, dist)) = best {
                if dist <= 26.0 {
                    active_idx = Some(i);
                } else if plot_rect.expand(12.0).contains(pos) {
                    // Fallback: grab nearest X point
                    let (t_click, _) = pos_to_temp_pct(pos);
                    active_idx = points
                        .iter()
                        .enumerate()
                        .min_by(|a, b| {
                            ((a.1.temp_c as f32) - t_click)
                                .abs()
                                .total_cmp(&((b.1.temp_c as f32) - t_click).abs())
                        })
                        .map(|(idx, _)| idx);
                }
                ui.ctx().data_mut(|d| d.insert_temp(drag_id, active_idx));
            }
        }
    }

    if response.dragged() {
        if let (Some(idx), Some(pos)) = (active_idx, response.interact_pointer_pos()) {
            if idx < points.len() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                let (raw_t, raw_p) = pos_to_temp_pct(pos);
                let min_t = if idx == 0 {
                    30
                } else {
                    points[idx - 1].temp_c.saturating_add(2).min(100)
                };
                let max_t = if idx + 1 >= points.len() {
                    100
                } else {
                    points[idx + 1].temp_c.saturating_sub(2).max(30)
                };
                let new_t = if min_t <= max_t {
                    raw_t.round().clamp(min_t as f32, max_t as f32) as u8
                } else {
                    points[idx].temp_c
                };
                let floor_p = if new_t >= 80 { 50 } else { 0 };
                let new_p = (raw_p.round().clamp(0.0, 100.0) as u8).max(floor_p);
                if points[idx].temp_c != new_t || points[idx].speed_pct != new_p {
                    points[idx].temp_c = new_t;
                    points[idx].speed_pct = new_p;
                    for pt in points.iter_mut().skip(idx + 1) {
                        if pt.speed_pct < new_p {
                            pt.speed_pct = new_p;
                        }
                    }
                    for j in (0..idx).rev() {
                        let floor = if points[j].temp_c >= 80 { 50 } else { 0 };
                        let cap = points[j + 1].speed_pct;
                        if points[j].speed_pct > cap {
                            points[j].speed_pct = cap.max(floor);
                        }
                    }
                    let mut running_max = 0u8;
                    for pt in points.iter_mut() {
                        if pt.temp_c >= 80 {
                            pt.speed_pct = pt.speed_pct.max(50);
                        }
                        if pt.speed_pct < running_max {
                            pt.speed_pct = running_max;
                        }
                        running_max = pt.speed_pct;
                    }
                    res.changed = true;
                }
            }
        }
    }

    if response.drag_stopped() {
        if active_idx.is_some() {
            res.committed = true;
        }
        active_idx = None;
        ui.ctx().data_mut(|d| d.remove::<Option<usize>>(drag_id));
    }

    // Build full polyline (clamped flat left to 30C and right to 100C)
    if !points.is_empty() {
        let mut poly: Vec<Pos2> = Vec::with_capacity(points.len() + 2);
        let first = points[0];
        let last = points[points.len() - 1];
        if first.temp_c > 30 {
            poly.push(Pos2::new(temp_to_x(30.0), pct_to_y(first.speed_pct as f32)));
        }
        for pt in points.iter() {
            poly.push(Pos2::new(temp_to_x(pt.temp_c as f32), pct_to_y(pt.speed_pct as f32)));
        }
        if last.temp_c < 100 {
            poly.push(Pos2::new(temp_to_x(100.0), pct_to_y(last.speed_pct as f32)));
        }

        let fill = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36);
        let base_y = plot_rect.bottom();
        for win in poly.windows(2) {
            let p1 = win[0];
            let p2 = win[1];
            if p2.x - p1.x <= 0.05 || (p1.y >= base_y - 0.5 && p2.y >= base_y - 0.5) {
                continue;
            }
            painter.add(Shape::convex_polygon(
                vec![p1, p2, Pos2::new(p2.x, base_y), Pos2::new(p1.x, base_y)],
                fill,
                Stroke::NONE,
            ));
        }
        painter.add(Shape::line(poly, Stroke::new(2.5_f32, accent)));
    }

    // Live operating point marker
    if let Some(cur_t) = live_temp_c {
        let clamped_t = cur_t.clamp(MIN_TEMP, MAX_TEMP);
        let lx = temp_to_x(clamped_t);
        painter.line_segment(
            [
                Pos2::new(lx, plot_rect.top()),
                Pos2::new(lx, plot_rect.bottom()),
            ],
            Stroke::new(
                1.5_f32,
                Color32::from_rgba_unmultiplied(STATUS_WARN.r(), STATUS_WARN.g(), STATUS_WARN.b(), 140),
            ),
        );
        let interp_pct = eval_linear_pct(points, clamped_t);
        let live_pt = Pos2::new(lx, pct_to_y(interp_pct));
        painter.circle_filled(live_pt, 5.0, STATUS_WARN);
        painter.circle_stroke(live_pt, 5.0, Stroke::new(1.5_f32, bg));
    }

    // Control handles + hover/drag tooltip
    let hover_pos = response.hover_pos();
    let mut highlight_idx = active_idx;
    if highlight_idx.is_none() {
        if let Some(h) = hover_pos {
            highlight_idx = points
                .iter()
                .enumerate()
                .map(|(i, pt)| {
                    let p = Pos2::new(temp_to_x(pt.temp_c as f32), pct_to_y(pt.speed_pct as f32));
                    (i, h.distance(p))
                })
                .filter(|&(_, d)| d <= 16.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            if highlight_idx.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
    }

    for (i, pt) in points.iter().enumerate() {
        let center = Pos2::new(temp_to_x(pt.temp_c as f32), pct_to_y(pt.speed_pct as f32));
        let highlighted = highlight_idx == Some(i);
        if highlighted {
            painter.circle_filled(
                center,
                11.0,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 64),
            );
            painter.circle_filled(center, 7.0, Color32::WHITE);
            painter.circle_stroke(center, 7.0, Stroke::new(2.2_f32, accent));
        } else {
            painter.circle_filled(center, 5.5, bg);
            painter.circle_stroke(center, 5.5, Stroke::new(2.0_f32, accent));
            painter.circle_filled(center, 2.2, Color32::WHITE);
        }
    }

    if let Some(hi) = highlight_idx {
        if let Some(pt) = points.get(hi) {
            let center = Pos2::new(temp_to_x(pt.temp_c as f32), pct_to_y(pt.speed_pct as f32));
            let est_rpm = ((max_fan_rpm as f32) * (pt.speed_pct as f32 / 100.0)).round() as u32;
            let text = format!("{}°C  ->  {}% (~{} RPM)", pt.temp_c, pt.speed_pct, est_rpm);
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), Color32::WHITE);
            let pad = Vec2::new(8.0, 4.0);
            let box_size = galley.size() + pad * 2.0;
            let tx = (center.x - box_size.x * 0.5)
                .clamp(plot_rect.left() + 4.0, plot_rect.right() - box_size.x - 4.0);
            let ty = if center.y - box_size.y - 14.0 >= plot_rect.top() {
                center.y - box_size.y - 14.0
            } else {
                center.y + 14.0
            };
            let pill = Rect::from_min_size(Pos2::new(tx, ty), box_size);
            painter.rect_filled(
                pill,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(15, 17, 22, 230),
            );
            painter.rect_stroke(
                pill,
                CornerRadius::same(6),
                Stroke::new(1.0_f32, accent),
                StrokeKind::Inside,
            );
            painter.galley(pill.min + pad, galley, Color32::WHITE);
        }
    }

    res
}

fn eval_linear_pct(points: &[FanCurvePoint], temp_c: f32) -> f32 {
    if points.is_empty() {
        return 50.0;
    }
    if temp_c <= points[0].temp_c as f32 {
        return points[0].speed_pct as f32;
    }
    let last = points[points.len() - 1];
    if temp_c >= last.temp_c as f32 {
        return last.speed_pct as f32;
    }
    for pair in points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if temp_c >= a.temp_c as f32 && temp_c <= b.temp_c as f32 {
            let span = (b.temp_c - a.temp_c) as f32;
            if span <= f32::EPSILON {
                return b.speed_pct as f32;
            }
            let r = (temp_c - a.temp_c as f32) / span;
            return a.speed_pct as f32 + r * (b.speed_pct as f32 - a.speed_pct as f32);
        }
    }
    50.0
}

pub fn battery_health_chart(
    ui: &mut Ui,
    records: &[BatteryHealthRecord],
    t: &CustomThemeConfig,
    health_label: &str,
    wear_label: &str,
) {
    if records.is_empty() {
        return;
    }

    let health_color = Color32::from_rgb(34, 197, 94);
    let wear_color = STATUS_WARN;
    let axis_text = muted_text_color(t);
    let bg = chart_bg_color(t);
    let border = rgb(t.card_border);
    let grid = subtle_grid_color(t);

    if let Some(latest) = records.last() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (lbl, val, color) in [
                (health_label, format!("{:.1}%", latest.health_pct), health_color),
                (wear_label, format!("{:.1}%", latest.wear_pct), wear_color),
            ] {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 4.0, color);
                    ui.label(egui::RichText::new(lbl).size(11.5).color(axis_text));
                    ui.label(egui::RichText::new(val).size(12.0).strong().color(color));
                });
            }
        });
    }

    let total_w = ui.available_width().max(240.0);
    let height = 148.0;
    let (outer_rect, response) =
        ui.allocate_exact_size(Vec2::new(total_w, height), egui::Sense::hover());
    let painter = ui.painter_at(outer_rect);

    painter.rect_filled(outer_rect, CornerRadius::same(8), bg);
    painter.rect_stroke(
        outer_rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );

    let plot_rect = Rect::from_min_max(
        Pos2::new(outer_rect.left() + 40.0, outer_rect.top() + 10.0),
        Pos2::new(
            (outer_rect.right() - 14.0).max(outer_rect.left() + 80.0),
            (outer_rect.bottom() - 22.0).max(outer_rect.top() + 50.0),
        ),
    );

    for pct in [0, 25, 50, 75, 100] {
        let frac = pct as f32 / 100.0;
        let py = plot_rect.bottom() - frac * plot_rect.height();
        painter.line_segment(
            [Pos2::new(plot_rect.left(), py), Pos2::new(plot_rect.right(), py)],
            Stroke::new(1.0_f32, grid),
        );
        painter.text(
            Pos2::new(plot_rect.left() - 6.0, py),
            Align2::RIGHT_CENTER,
            format!("{pct}%"),
            FontId::proportional(10.0),
            axis_text,
        );
    }

    let count = records.len();
    let map_x = |idx: usize| -> f32 {
        if count <= 1 {
            plot_rect.center().x
        } else {
            plot_rect.left() + (idx as f32 / (count - 1) as f32) * plot_rect.width()
        }
    };
    let map_y = |pct: f32| -> f32 {
        plot_rect.bottom() - (pct.clamp(0.0, 100.0) / 100.0) * plot_rect.height()
    };

    let health_pts: Vec<Pos2> = records
        .iter()
        .enumerate()
        .map(|(i, r)| Pos2::new(map_x(i), map_y(r.health_pct)))
        .collect();
    let wear_pts: Vec<Pos2> = records
        .iter()
        .enumerate()
        .map(|(i, r)| Pos2::new(map_x(i), map_y(r.wear_pct)))
        .collect();

    if health_pts.len() >= 2 {
        painter.add(Shape::line(health_pts.clone(), Stroke::new(2.2_f32, health_color)));
        painter.add(Shape::line(wear_pts.clone(), Stroke::new(2.0_f32, wear_color)));
    }
    for (p_h, p_w) in health_pts.iter().zip(wear_pts.iter()) {
        painter.circle_filled(*p_h, 3.5, health_color);
        painter.circle_filled(*p_w, 3.0, wear_color);
    }

    // X-axis date labels (first, middle if >2, last)
    if let Some(first) = records.first() {
        painter.text(
            Pos2::new(plot_rect.left(), plot_rect.bottom() + 4.0),
            Align2::LEFT_TOP,
            &first.date_label,
            FontId::proportional(10.0),
            axis_text,
        );
    }
    if count >= 3 {
        let mid_idx = count / 2;
        painter.text(
            Pos2::new(map_x(mid_idx), plot_rect.bottom() + 4.0),
            Align2::CENTER_TOP,
            &records[mid_idx].date_label,
            FontId::proportional(10.0),
            axis_text,
        );
    }
    if count >= 2 {
        if let Some(last) = records.last() {
            painter.text(
                Pos2::new(plot_rect.right(), plot_rect.bottom() + 4.0),
                Align2::RIGHT_TOP,
                &last.date_label,
                FontId::proportional(10.0),
                axis_text,
            );
        }
    }

    if let Some(hover) = response.hover_pos() {
        if plot_rect.contains(hover) {
            let nearest_idx = if count <= 1 {
                0
            } else {
                let norm = ((hover.x - plot_rect.left()) / plot_rect.width().max(1.0))
                    .clamp(0.0, 1.0);
                ((norm * (count - 1) as f32).round() as usize).min(count - 1)
            };
            let rec = &records[nearest_idx];
            let px = map_x(nearest_idx);
            painter.line_segment(
                [Pos2::new(px, plot_rect.top()), Pos2::new(px, plot_rect.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(220, 225, 235, 110)),
            );
            painter.circle_filled(Pos2::new(px, map_y(rec.health_pct)), 5.0, health_color);
            painter.circle_filled(Pos2::new(px, map_y(rec.wear_pct)), 4.5, wear_color);

            let tip_txt = format!(
                "{}  ·  {health_label}: {:.1}%  ·  {wear_label}: {:.1}%  ({}/{} mWh)",
                rec.date_label,
                rec.health_pct,
                rec.wear_pct,
                rec.full_charge_capacity_mwh,
                rec.design_capacity_mwh
            );
            let galley =
                painter.layout_no_wrap(tip_txt, FontId::proportional(10.8), Color32::WHITE);
            let pad = Vec2::new(7.0, 3.0);
            let pill_w = galley.size().x + pad.x * 2.0;
            let pill_x = (px - pill_w * 0.5)
                .clamp(plot_rect.left() + 2.0, (plot_rect.right() - pill_w - 2.0).max(plot_rect.left() + 2.0));
            let pill = Rect::from_min_size(
                Pos2::new(pill_x, plot_rect.top() + 4.0),
                galley.size() + pad * 2.0,
            );
            painter.rect_filled(
                pill,
                CornerRadius::same(5),
                Color32::from_rgba_unmultiplied(15, 17, 22, 225),
            );
            painter.galley(pill.min + pad, galley, Color32::WHITE);
        }
    }
}
