use crate::theme::{
    card_frame, feature_toggle_button, pill_button, section_title, status_badge, status_ok,
    status_warn,
};
use crate::ui::charts::interactive_fan_curve;
use crate::ui::control_committed;
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::thermal::{
    effective_fan_curve_temp_c, MAX_GUARD_CPU_TEMP_C, MAX_GUARD_SUSTAIN_SECS, MIN_GUARD_CPU_TEMP_C,
    MIN_GUARD_SUSTAIN_SECS,
};
use omvix_core::{
    AppConfig, FanCurve, FanCurvePoint, FanMode, I18n, PerformanceMode, WindowsPowerOverlay,
};

#[derive(Debug, Default, Clone)]
pub struct PerformanceDraftState {
    pub curve_import_input: String,
    pub curve_status_msg: Option<(String, bool)>,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    draft: &mut PerformanceDraftState,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();

    ui.heading(i18n.t("nav.performance_fans"));
    ui.add_space(6.0);

    if !st.fan_conflict_services.is_empty() {
        card_frame(t).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, &st.fan_conflict_services.join(", "), status_warn(t));
                ui.label(i18n.t("fan.conflict_service_warning"));
            });
        });
        ui.add_space(6.0);
    }

    // Thermal / Performance Mode Card + Windows Power Mode Sync
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
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("thermal.sync_windows_power_mode"),
                &mut cfg.thermal_fan.sync_windows_power_mode,
                &i18n.t("common.enabled"),
                t,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                if cfg.thermal_fan.sync_windows_power_mode {
                    on_cmd(WorkerCommand::SetPerformanceMode(
                        cfg.thermal_fan.performance_mode,
                    ));
                }
            }
            if let Some(overlay) = st.active_win_power_mode {
                let ov_key = match overlay {
                    WindowsPowerOverlay::BestPowerEfficiency => "thermal.win_power_efficiency",
                    WindowsPowerOverlay::Balanced => "thermal.win_power_balanced",
                    WindowsPowerOverlay::BestPerformance => "thermal.win_power_performance",
                };
                status_badge(
                    ui,
                    &i18n.t_fmt("thermal.win_power_active", &[("mode", &i18n.t(ov_key))]),
                    status_ok(t),
                );
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
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("fan.manual_speed"));
                let resp =
                    ui.add(Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100).suffix("%"));
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
                    status_warn(t),
                    i18n.t_fmt(
                        "fan.no_cpu_sensor_notice",
                        &[("temp", &cfg.thermal_fan.guard_cpu_temp_c.to_string())],
                    ),
                );
                ui.add_space(4.0);
            }
            ui.weak(RichText::new(i18n.t("fan.curve_drag_hint")).size(11.5));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("fan.curve_presets"));
                for (key, preset) in [
                    ("fan.preset_quiet", FanCurve::preset_quiet()),
                    ("fan.preset_balanced", FanCurve::preset_balanced()),
                    ("fan.preset_aggressive", FanCurve::preset_aggressive()),
                    ("fan.preset_ogh_like", FanCurve::preset_ogh_like()),
                ] {
                    let selected = cfg.thermal_fan.custom_curve == preset;
                    if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                        cfg.thermal_fan.custom_curve = preset;
                        draft.curve_status_msg = None;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                }
            });
            ui.add_space(4.0);
            cfg.thermal_fan
                .custom_curve
                .points
                .sort_by_key(|p| p.temp_c);

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
                let _ = cfg.thermal_fan.custom_curve.normalize();
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }

            ui.add_space(6.0);
            ui.collapsing(i18n.t("fan.curve_manual_table"), |ui| {
                let mut remove_idx: Option<usize> = None;
                let mut table_committed = false;
                let num_pts = cfg.thermal_fan.custom_curve.points.len();
                for (idx, pt) in cfg.thermal_fan.custom_curve.points.iter_mut().enumerate() {
                    ui.horizontal_wrapped(|ui| {
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
                        let speed_resp =
                            ui.add(Slider::new(&mut pt.speed_pct, 0..=100).suffix("%"));
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
                    let _ = cfg.thermal_fan.custom_curve.normalize();
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });

            ui.add_space(6.0);
            let num_pts = cfg.thermal_fan.custom_curve.points.len();
            ui.horizontal_wrapped(|ui| {
                if num_pts < 8 && ui.button(i18n.t("fan.add_point")).clicked() {
                    cfg.thermal_fan.custom_curve.points.push(FanCurvePoint {
                        temp_c: 75,
                        speed_pct: 75,
                    });
                    let _ = cfg.thermal_fan.custom_curve.normalize();
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
                if ui.button(i18n.t("fan.reset_curve")).clicked() {
                    cfg.thermal_fan.custom_curve = FanCurve::default();
                    draft.curve_status_msg = None;
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });

            ui.add_space(6.0);
            ui.collapsing(i18n.t("fan.curve_share"), |ui| {
                let mut code = cfg.thermal_fan.custom_curve.encode_share_code();
                let code_w = (ui.available_width() - 190.0).clamp(160.0, 240.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut code)
                            .desired_width(code_w)
                            .interactive(false),
                    );
                    if ui.button(i18n.t("fan.curve_copy_code")).clicked() {
                        ui.ctx().copy_text(code.clone());
                        draft.curve_status_msg = Some((i18n.t("fan.curve_copied"), false));
                    }
                    if ui.button(i18n.t("fan.curve_copy_json")).clicked() {
                        if let Ok(json) =
                            serde_json::to_string_pretty(&cfg.thermal_fan.custom_curve)
                        {
                            ui.ctx().copy_text(json);
                            draft.curve_status_msg = Some((i18n.t("fan.curve_copied"), false));
                        }
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.curve_import_input)
                            .desired_width(code_w)
                            .hint_text(i18n.t("fan.curve_import_placeholder")),
                    );
                    if pill_button(ui, &i18n.t("fan.curve_import_btn"), false, t).clicked() {
                        match FanCurve::decode_share_code(&draft.curve_import_input) {
                            Ok(imported) => {
                                cfg.thermal_fan.custom_curve = imported;
                                draft.curve_import_input.clear();
                                draft.curve_status_msg =
                                    Some((i18n.t("fan.curve_imported"), false));
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            }
                            Err(err) => {
                                draft.curve_status_msg = Some((err.to_string(), true));
                            }
                        }
                    }
                });

                if let Some((msg, is_err)) = &draft.curve_status_msg {
                    ui.colored_label(
                        if *is_err {
                            crate::theme::status_err(t)
                        } else {
                            crate::theme::status_ok(t)
                        },
                        msg,
                    );
                }
            });
        }

        ui.add_space(6.0);
        ui.collapsing(i18n.t("fan.advanced_settings"), |ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("fan.keep_alive"),
                &mut cfg.thermal_fan.keep_alive_enabled,
                &i18n.t("common.enabled"),
                t,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            ui.weak(RichText::new(i18n.t("fan.keep_alive_desc")).size(11.0));
            ui.add_space(6.0);

            ui.horizontal_wrapped(|ui| {
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
            ui.add_space(6.0);

            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("fan.guard_cpu_temp"));
                let temp_resp = ui.add(
                    Slider::new(
                        &mut cfg.thermal_fan.guard_cpu_temp_c,
                        MIN_GUARD_CPU_TEMP_C..=MAX_GUARD_CPU_TEMP_C,
                    )
                    .suffix("°C"),
                );
                if control_committed(ui, "perf_guard_cpu_temp", &temp_resp) {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("fan.guard_sustain_secs"));
                let dur_resp = ui.add(
                    Slider::new(
                        &mut cfg.thermal_fan.guard_sustain_secs,
                        MIN_GUARD_SUSTAIN_SECS..=MAX_GUARD_SUSTAIN_SECS,
                    )
                    .suffix(" s"),
                );
                if control_committed(ui, "perf_guard_sustain", &dur_resp) {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
            ui.weak(RichText::new(i18n.t("fan.guard_desc")).size(11.0));
        });
    });
}
