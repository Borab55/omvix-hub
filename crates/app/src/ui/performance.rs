use crate::theme::{card_frame, pill_button, section_title};
use crate::ui::charts::interactive_fan_curve;
use crate::ui::control_committed;
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::thermal::effective_fan_curve_temp_c;
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

    ui.heading(i18n.t("nav.performance_fans"));
    ui.add_space(6.0);

    // Thermal / Performance Mode Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("thermal.mode"), t);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (PerformanceMode::Eco, "thermal.mode_eco"),
                (PerformanceMode::Quiet, "thermal.mode_quiet"),
                (PerformanceMode::Balanced, "thermal.mode_balanced"),
                (PerformanceMode::Performance, "thermal.mode_performance"),
            ] {
                let selected = cfg.thermal_fan.performance_mode == mode;
                if pill_button(ui, &i18n.t(key), selected, t).clicked() {
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
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
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
                                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
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
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
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
                                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
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
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (FanMode::Auto, "fan.mode_auto"),
                (FanMode::Max, "fan.mode_max"),
                (FanMode::Manual, "fan.mode_manual"),
                (FanMode::CustomCurve, "fan.mode_curve"),
            ] {
                let selected = cfg.thermal_fan.fan_mode == mode;
                if pill_button(ui, &i18n.t(key), selected, t).clicked() {
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
                let resp = ui.add(
                    Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100).suffix("%"),
                );
                if control_committed(ui, "perf_manual_fan", &resp) {
                    on_save();
                    on_cmd(WorkerCommand::SetFanMode {
                        mode: FanMode::Manual,
                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                    });
                }
            });
        }

        if cfg.thermal_fan.fan_mode == FanMode::CustomCurve {
            ui.add_space(4.0);
            if st.telemetry.cpu_temp_c.is_none() {
                ui.colored_label(
                    crate::theme::STATUS_WARN,
                    i18n.t("fan.no_cpu_sensor_notice"),
                );
                ui.add_space(4.0);
            }
            ui.weak(RichText::new(i18n.t("fan.curve_drag_hint")).size(11.5));
            ui.add_space(4.0);
            cfg.thermal_fan.custom_curve.points.sort_by_key(|p| p.temp_c);

            let live_temp = effective_fan_curve_temp_c(
                st.telemetry.cpu_temp_c,
                st.telemetry.gpu_temp_c,
                st.telemetry.chassis_temp_c,
                st.telemetry.cpu_power_w,
            );
            let max_rpm = st.telemetry.fan_max_rpms.first().copied().unwrap_or(5800);

            let edit = interactive_fan_curve(
                ui,
                &mut cfg.thermal_fan.custom_curve.points,
                live_temp,
                max_rpm,
                t,
            );
            if edit.committed {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }

            ui.add_space(6.0);
            ui.collapsing(i18n.t("fan.curve_manual_table"), |ui| {
                let mut remove_idx: Option<usize> = None;
                let mut table_committed = false;
                let num_pts = cfg.thermal_fan.custom_curve.points.len();
                for (idx, pt) in cfg.thermal_fan.custom_curve.points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("#{}", idx + 1));
                        ui.label(i18n.t("fan.curve_temp"));
                        let temp_resp = ui.add(
                            egui::DragValue::new(&mut pt.temp_c)
                                .range(30..=100)
                                .suffix("°C"),
                        );
                        if control_committed(ui, ("curve_pt_temp", idx), &temp_resp) {
                            table_committed = true;
                        }
                        ui.label(i18n.t("fan.curve_speed"));
                        let speed_resp = ui.add(Slider::new(&mut pt.speed_pct, 0..=100).suffix("%"));
                        if control_committed(ui, ("curve_pt_speed", idx), &speed_resp) {
                            table_committed = true;
                        }
                        if num_pts > 2 && ui.button("×").clicked() {
                            remove_idx = Some(idx);
                            table_committed = true;
                        }
                    });
                }
                if let Some(idx) = remove_idx {
                    cfg.thermal_fan.custom_curve.points.remove(idx);
                }
                if table_committed {
                    cfg.thermal_fan.custom_curve.points.sort_by_key(|p| p.temp_c);
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });

            ui.add_space(6.0);
            let num_pts = cfg.thermal_fan.custom_curve.points.len();
            ui.horizontal(|ui| {
                if num_pts < 8 && ui.button(i18n.t("fan.add_point")).clicked() {
                    cfg.thermal_fan
                        .custom_curve
                        .points
                        .push(FanCurvePoint { temp_c: 75, speed_pct: 75 });
                    cfg.thermal_fan.custom_curve.points.sort_by_key(|p| p.temp_c);
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
                if ui.button(i18n.t("fan.reset_curve")).clicked() {
                    cfg.thermal_fan.custom_curve = FanCurve::default();
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
        }

        ui.add_space(6.0);
        ui.collapsing(i18n.t("fan.advanced_settings"), |ui| {
            if ui
                .checkbox(&mut cfg.thermal_fan.keep_alive_enabled, i18n.t("fan.keep_alive"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            ui.weak(RichText::new(i18n.t("fan.keep_alive_desc")).size(11.0));
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label(i18n.t("fan.hysteresis"));
                let resp = ui.add(
                    Slider::new(&mut cfg.thermal_fan.custom_curve.hysteresis_c, 1..=10)
                        .suffix("°C"),
                );
                if control_committed(ui, "perf_fan_hysteresis", &resp) {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
            ui.weak(RichText::new(i18n.t("fan.hysteresis_desc")).size(11.0));
        });
    });
}
