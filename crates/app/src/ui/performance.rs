use crate::theme::{accent_color, card_frame, section_title};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Slider, Ui};
use egui_plot::{Line, Plot, PlotPoints, Points};
use omvix_core::{AppConfig, FanCurve, FanCurvePoint, FanMode, I18n, PerformanceMode};

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    ui.heading(i18n.t("nav.performance_fans"));
    ui.add_space(6.0);

    // Thermal / Performance Mode Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("thermal.mode"), t);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (PerformanceMode::Eco, "thermal.mode_eco"),
                (PerformanceMode::Quiet, "thermal.mode_quiet"),
                (PerformanceMode::Balanced, "thermal.mode_balanced"),
                (PerformanceMode::Performance, "thermal.mode_performance"),
            ] {
                let selected = cfg.thermal_fan.performance_mode == mode;
                if ui.selectable_label(selected, i18n.t(key)).clicked() {
                    cfg.thermal_fan.performance_mode = mode;
                    on_save();
                    on_cmd(WorkerCommand::SetPerformanceMode(mode));
                }
            }
        });
    });

    ui.add_space(8.0);

    // AC / Battery Auto-Profile Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, i18n.t("thermal.auto_profile"), t);
            if ui
                .checkbox(&mut cfg.auto_profile.enabled, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });

        ui.add_enabled_ui(cfg.auto_profile.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label(i18n.t("thermal.ac_profile"));
                egui::ComboBox::from_id_salt("ac_perf_combo")
                    .selected_text(format!("{:?}", cfg.auto_profile.ac_performance_mode))
                    .show_ui(ui, |ui| {
                        for m in [
                            PerformanceMode::Eco,
                            PerformanceMode::Quiet,
                            PerformanceMode::Balanced,
                            PerformanceMode::Performance,
                        ] {
                            if ui
                                .selectable_value(
                                    &mut cfg.auto_profile.ac_performance_mode,
                                    m,
                                    format!("{m:?}"),
                                )
                                .changed()
                            {
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            }
                        }
                    });

                ui.add_space(12.0);
                ui.label(i18n.t("thermal.dc_profile"));
                egui::ComboBox::from_id_salt("dc_perf_combo")
                    .selected_text(format!("{:?}", cfg.auto_profile.dc_performance_mode))
                    .show_ui(ui, |ui| {
                        for m in [
                            PerformanceMode::Eco,
                            PerformanceMode::Quiet,
                            PerformanceMode::Balanced,
                            PerformanceMode::Performance,
                        ] {
                            if ui
                                .selectable_value(
                                    &mut cfg.auto_profile.dc_performance_mode,
                                    m,
                                    format!("{m:?}"),
                                )
                                .changed()
                            {
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            }
                        }
                    });
            });

            if let Some(disp) = &st.display_mode {
                ui.horizontal(|ui| {
                    ui.label("AC Hz:");
                    egui::ComboBox::from_id_salt("ac_hz_combo")
                        .selected_text(
                            cfg.auto_profile
                                .ac_refresh_hz
                                .map(|h| format!("{h} Hz"))
                                .unwrap_or_else(|| "—".into()),
                        )
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut cfg.auto_profile.ac_refresh_hz, None, "—")
                                .changed()
                            {
                                on_save();
                            }
                            for &hz in &disp.supported_hz {
                                if ui
                                    .selectable_value(
                                        &mut cfg.auto_profile.ac_refresh_hz,
                                        Some(hz),
                                        format!("{hz} Hz"),
                                    )
                                    .changed()
                                {
                                    on_save();
                                }
                            }
                        });

                    ui.add_space(12.0);
                    ui.label("Battery Hz:");
                    egui::ComboBox::from_id_salt("dc_hz_combo")
                        .selected_text(
                            cfg.auto_profile
                                .dc_refresh_hz
                                .map(|h| format!("{h} Hz"))
                                .unwrap_or_else(|| "—".into()),
                        )
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut cfg.auto_profile.dc_refresh_hz, None, "—")
                                .changed()
                            {
                                on_save();
                            }
                            for &hz in &disp.supported_hz {
                                if ui
                                    .selectable_value(
                                        &mut cfg.auto_profile.dc_refresh_hz,
                                        Some(hz),
                                        format!("{hz} Hz"),
                                    )
                                    .changed()
                                {
                                    on_save();
                                }
                            }
                        });
                });
            }

            if ui
                .checkbox(
                    &mut cfg.auto_profile.dim_keyboard_on_battery,
                    "Turn off keyboard backlight on battery",
                )
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });
    });

    ui.add_space(8.0);

    // Fan Control Card + Interactive Custom Curve
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("fan.mode"), t);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (FanMode::Auto, "fan.mode_auto"),
                (FanMode::Max, "fan.mode_max"),
                (FanMode::Manual, "fan.mode_manual"),
                (FanMode::CustomCurve, "fan.mode_curve"),
            ] {
                let selected = cfg.thermal_fan.fan_mode == mode;
                if ui.selectable_label(selected, i18n.t(key)).clicked() {
                    cfg.thermal_fan.fan_mode = mode;
                    on_save();
                    on_cmd(WorkerCommand::SetFanMode {
                        mode,
                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                    });
                }
            }
        });

        ui.add_space(6.0);
        if cfg.thermal_fan.fan_mode == FanMode::Manual {
            ui.horizontal(|ui| {
                ui.label(i18n.t("fan.manual_speed"));
                ui.add(Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100).suffix("%"));
                if ui.button(i18n.t("common.apply")).clicked() {
                    on_save();
                    on_cmd(WorkerCommand::SetFanMode {
                        mode: FanMode::Manual,
                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                    });
                }
            });
        }

        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut cfg.thermal_fan.keep_alive_enabled, i18n.t("fan.keep_alive"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            ui.add_space(10.0);
            ui.label(i18n.t("fan.hysteresis"));
            if ui
                .add(Slider::new(&mut cfg.thermal_fan.custom_curve.hysteresis_c, 1..=10).suffix("°C"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });

        if cfg.thermal_fan.fan_mode == FanMode::CustomCurve {
            ui.add_space(8.0);
            cfg.thermal_fan.custom_curve.points.sort_by_key(|p| p.temp_c);

            let line_pts: PlotPoints<'_> = cfg
                .thermal_fan
                .custom_curve
                .points
                .iter()
                .map(|p| [p.temp_c as f64, p.speed_pct as f64])
                .collect();
            let scatter_pts: PlotPoints<'_> = cfg
                .thermal_fan
                .custom_curve
                .points
                .iter()
                .map(|p| [p.temp_c as f64, p.speed_pct as f64])
                .collect();

            Plot::new("fan_curve_plot")
                .height(190.0)
                .include_x(25.0)
                .include_x(105.0)
                .include_y(0.0)
                .include_y(105.0)
                .allow_zoom(false)
                .allow_drag(false)
                .show(ui, |plot_ui| {
                    plot_ui.line(Line::new(line_pts).color(accent).width(2.0_f32));
                    plot_ui.points(
                        Points::new(scatter_pts)
                            .color(Color32::from_rgb(245, 158, 11))
                            .radius(4.5_f32),
                    );
                });

            ui.add_space(6.0);
            let mut remove_idx: Option<usize> = None;
            let num_pts = cfg.thermal_fan.custom_curve.points.len();
            for (idx, pt) in cfg.thermal_fan.custom_curve.points.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("#{}", idx + 1));
                    ui.label(i18n.t("fan.curve_temp"));
                    ui.add(egui::DragValue::new(&mut pt.temp_c).range(30..=100).suffix("°C"));
                    ui.label(i18n.t("fan.curve_speed"));
                    ui.add(Slider::new(&mut pt.speed_pct, 0..=100).suffix("%"));
                    if num_pts > 2 && ui.button("×").clicked() {
                        remove_idx = Some(idx);
                    }
                });
            }
            if let Some(idx) = remove_idx {
                cfg.thermal_fan.custom_curve.points.remove(idx);
            }

            ui.horizontal(|ui| {
                if ui.button(i18n.t("fan.add_point")).clicked() && num_pts < 10 {
                    cfg.thermal_fan
                        .custom_curve
                        .points
                        .push(FanCurvePoint { temp_c: 75, speed_pct: 75 });
                }
                if ui.button(i18n.t("fan.reset_curve")).clicked() {
                    cfg.thermal_fan.custom_curve = FanCurve::default();
                }
                if ui.button(RichText::new(i18n.t("common.apply")).strong()).clicked() {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
        }
    });
}
