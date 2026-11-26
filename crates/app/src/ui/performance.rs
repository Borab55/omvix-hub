use crate::theme::{
    accent_color, card_frame, feature_toggle_button, pill_button, section_title, status_badge,
    status_err, status_ok, status_warn,
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
    AppConfig, CpuBoostMode, CustomThemeConfig, FanCurve, FanCurvePoint, FanMode, GpuMode, I18n,
    PerformanceMode, WindowsPowerOverlay,
};

#[derive(Debug, Default, Clone)]
pub struct PerformanceDraftState {
    pub curve_import_input: String,
    pub curve_status_msg: Option<(String, bool)>,
    pub gpu_curve_import_input: String,
    pub gpu_curve_status_msg: Option<(String, bool)>,
    pub pending_gpu_mode: Option<GpuMode>,
    pub gpu_reboot_dismissed: bool,
}

pub(crate) fn render_fan_mode_selector(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    effective_fan: FanMode,
    i18n: &I18n,
    t: &CustomThemeConfig,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    ui.horizontal_wrapped(|ui| {
        for (mode, key) in [
            (FanMode::Auto, "fan.mode_auto"),
            (FanMode::Max, "fan.mode_max"),
            (FanMode::Manual, "fan.mode_manual"),
            (FanMode::CustomCurve, "fan.mode_curve"),
        ] {
            let selected = effective_fan == mode;
            if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                cfg.thermal_fan.fan_mode = mode;
                on_save();
                let cpu_pct = cfg.thermal_fan.manual_speed_pct;
                let gpu_pct = if cfg.thermal_fan.link_cpu_gpu_fans {
                    cpu_pct
                } else {
                    cfg.thermal_fan.gpu_manual_speed_pct
                };
                on_cmd(WorkerCommand::SetFanMode {
                    mode,
                    manual_speed_pct: Some(cpu_pct),
                    gpu_manual_speed_pct: Some(gpu_pct),
                });
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_fan_curve_editor(
    ui: &mut Ui,
    id_salt: &str,
    curve: &mut FanCurve,
    live_temp: Option<f32>,
    max_rpm: u32,
    import_input: &mut String,
    status_msg: &mut Option<(String, bool)>,
    i18n: &I18n,
    t: &CustomThemeConfig,
) -> bool {
    let mut changed = false;

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
            let selected = *curve == preset;
            if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                *curve = preset;
                *status_msg = None;
                changed = true;
            }
        }
    });
    ui.add_space(4.0);

    let edit = interactive_fan_curve(
        ui,
        id_salt,
        &mut curve.points,
        live_temp,
        max_rpm,
        &i18n.t("fan.curve_safety_floor"),
        &i18n.t("fan.curve_quiet_zone"),
        t,
    );
    if edit.committed {
        let _ = curve.normalize();
        changed = true;
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(i18n.t("fan.hysteresis"));
        let resp = ui.add(Slider::new(&mut curve.hysteresis_c, 1..=10).suffix("°C"));
        if control_committed(ui, (id_salt, "hysteresis"), &resp) {
            let _ = curve.normalize();
            changed = true;
        }
    });
    ui.weak(RichText::new(i18n.t("fan.hysteresis_desc")).size(11.0));

    ui.add_space(4.0);
    let num_pts = curve.points.len();
    ui.horizontal_wrapped(|ui| {
        if num_pts < 8 && ui.button(i18n.t("fan.add_point")).clicked() {
            curve.points.push(FanCurvePoint {
                temp_c: 75,
                speed_pct: 75,
            });
            let _ = curve.normalize();
            changed = true;
        }
        if ui.button(i18n.t("fan.reset_curve")).clicked() {
            *curve = FanCurve::default();
            *status_msg = None;
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.push_id((id_salt, "table_collapse"), |ui| {
        ui.collapsing(i18n.t("fan.curve_manual_table"), |ui| {
            let mut remove_idx: Option<usize> = None;
            let mut table_committed = false;
            let pts_len = curve.points.len();
            for (idx, pt) in curve.points.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("#{}", idx + 1));
                    ui.label(i18n.t("fan.curve_temp"));
                    let temp_resp = ui.add(
                        egui::DragValue::new(&mut pt.temp_c)
                            .range(30..=100)
                            .suffix("°C"),
                    );
                    if control_committed(ui, (id_salt, "pt_temp", idx), &temp_resp) {
                        table_committed = true;
                    }
                    ui.label(i18n.t("fan.curve_speed"));
                    let speed_resp = ui.add(Slider::new(&mut pt.speed_pct, 0..=100).suffix("%"));
                    if control_committed(ui, (id_salt, "pt_speed", idx), &speed_resp) {
                        table_committed = true;
                    }
                    if pts_len > 2 && ui.button("×").clicked() {
                        remove_idx = Some(idx);
                        table_committed = true;
                    }
                });
            }
            if let Some(idx) = remove_idx {
                curve.points.remove(idx);
            }
            if table_committed {
                let _ = curve.normalize();
                changed = true;
            }
        });
    });

    ui.add_space(4.0);
    ui.push_id((id_salt, "share_collapse"), |ui| {
        ui.collapsing(i18n.t("fan.curve_share"), |ui| {
            let mut code = curve.encode_share_code();
            let code_w = (ui.available_width() - 190.0).clamp(160.0, 240.0);
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut code)
                        .desired_width(code_w)
                        .interactive(false),
                );
                if ui.button(i18n.t("fan.curve_copy_code")).clicked() {
                    ui.ctx().copy_text(code.clone());
                    *status_msg = Some((i18n.t("fan.curve_copied"), false));
                }
                if ui.button(i18n.t("fan.curve_copy_json")).clicked() {
                    if let Ok(json) = serde_json::to_string_pretty(curve) {
                        ui.ctx().copy_text(json);
                        *status_msg = Some((i18n.t("fan.curve_copied"), false));
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(import_input)
                        .desired_width(code_w)
                        .hint_text(i18n.t("fan.curve_import_placeholder")),
                );
                if pill_button(ui, &i18n.t("fan.curve_import_btn"), false, t).clicked() {
                    match FanCurve::decode_share_code(import_input) {
                        Ok(imported) => {
                            *curve = imported;
                            import_input.clear();
                            *status_msg = Some((i18n.t("fan.curve_imported"), false));
                            changed = true;
                        }
                        Err(err) => {
                            *status_msg = Some((err.to_string(), true));
                        }
                    }
                }
            });

            if let Some((msg, is_err)) = status_msg {
                ui.colored_label(if *is_err { status_err(t) } else { status_ok(t) }, msg);
            }
        });
    });

    changed
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
    let accent = accent_color(t);
    let ok_c = status_ok(t);
    let warn_c = status_warn(t);
    let (effective_perf, effective_fan) = if st.active_app_profile.is_some() {
        (st.effective_performance_mode, st.effective_fan_mode)
    } else {
        (cfg.thermal_fan.performance_mode, cfg.thermal_fan.fan_mode)
    };

    ui.horizontal_wrapped(|ui| {
        ui.heading(i18n.t("drawer.sub_cpu"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cpu_rpm = st
                .telemetry
                .fan_rpms
                .first()
                .map(|r| format!("{r} RPM"))
                .unwrap_or_else(|| "—".into());
            status_badge(ui, &format!("Fan: {cpu_rpm}"), accent);
            if let Some(w) = st.telemetry.cpu_power_w {
                status_badge(ui, &format!("{w:.0} W"), ok_c);
            }
            if let Some(temp) = st.telemetry.cpu_temp_c {
                status_badge(ui, &format!("{temp:.0}°C"), warn_c);
            }
        });
    });
    ui.add_space(6.0);

    if !st.fan_conflict_services.is_empty() {
        card_frame(t).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, &st.fan_conflict_services.join(", "), warn_c);
                ui.label(i18n.t("fan.conflict_service_warning"));
            });
        });
        ui.add_space(6.0);
    }

    // Card 1: Performance Mode + CPU Turbo Boost + CPU Power Limits
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
                let selected = effective_perf == mode;
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
                    ok_c,
                );
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        section_title(ui, i18n.t("tuning.cpu_boost"), t);
        ui.weak(RichText::new(i18n.t("tuning.cpu_boost_desc")).size(11.0));
        ui.add_space(3.0);
        let cur_boost = st
            .cpu_boost
            .or(cfg.tuning.cpu_boost_mode)
            .unwrap_or(CpuBoostMode::Aggressive);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (CpuBoostMode::Disabled, "cockpit.boost_off"),
                (CpuBoostMode::Enabled, "cockpit.boost_on"),
                (CpuBoostMode::Aggressive, "cockpit.boost_aggressive"),
                (
                    CpuBoostMode::EfficientAggressive,
                    "profiles.boost_efficient",
                ),
            ] {
                if pill_button(ui, &i18n.t(key), cur_boost == mode, t).clicked() {
                    cfg.tuning.cpu_boost_mode = Some(mode);
                    on_save();
                    on_cmd(WorkerCommand::SetCpuBoost(mode));
                }
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        section_title(ui, i18n.t("tuning.power_limits_title"), t);
        ui.weak(RichText::new(i18n.t("tuning.power_limits_desc")).size(11.0));
        if !st.board_known {
            ui.colored_label(warn_c, i18n.t("tuning.power_limit_unknown_board"));
        }
        ui.add_space(4.0);

        ui.add_enabled_ui(st.board_known, |ui| {
            let mut pl1_enabled = cfg.tuning.pl1_watts.is_some();
            ui.horizontal_wrapped(|ui| {
                if feature_toggle_button(
                    ui,
                    &i18n.t("tuning.power_limit_pl1"),
                    &mut pl1_enabled,
                    &i18n.t("common.enabled"),
                    t,
                )
                .changed()
                {
                    if pl1_enabled {
                        cfg.tuning.pl1_watts = Some(55);
                        on_save();
                    } else {
                        cfg.tuning.pl1_watts = None;
                        on_save();
                        on_cmd(WorkerCommand::RestoreDefaultPowerLimits {
                            keep_pl1: None,
                            keep_pl2: cfg.tuning.pl2_watts,
                        });
                    }
                }
                if let Some(ref mut pl1) = cfg.tuning.pl1_watts {
                    ui.add(Slider::new(pl1, 15..=175).suffix(" W"));
                } else {
                    ui.weak(i18n.t("tuning.default_bios"));
                }
            });

            let mut pl2_enabled = cfg.tuning.pl2_watts.is_some();
            ui.horizontal_wrapped(|ui| {
                if feature_toggle_button(
                    ui,
                    &i18n.t("tuning.power_limit_pl2"),
                    &mut pl2_enabled,
                    &i18n.t("common.enabled"),
                    t,
                )
                .changed()
                {
                    if pl2_enabled {
                        cfg.tuning.pl2_watts = Some(115);
                        on_save();
                    } else {
                        cfg.tuning.pl2_watts = None;
                        on_save();
                        on_cmd(WorkerCommand::RestoreDefaultPowerLimits {
                            keep_pl1: cfg.tuning.pl1_watts,
                            keep_pl2: None,
                        });
                    }
                }
                if let Some(ref mut pl2) = cfg.tuning.pl2_watts {
                    ui.add(Slider::new(pl2, 20..=200).suffix(" W"));
                } else {
                    ui.weak(i18n.t("tuning.default_bios"));
                }
            });

            let has_any_pl = cfg.tuning.pl1_watts.is_some() || cfg.tuning.pl2_watts.is_some();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(has_any_pl, egui::Button::new(i18n.t("common.apply")))
                    .clicked()
                {
                    on_save();
                    on_cmd(WorkerCommand::SetPowerLimits {
                        pl1: cfg.tuning.pl1_watts,
                        pl2: cfg.tuning.pl2_watts,
                    });
                }
                if ui
                    .add_enabled(
                        has_any_pl,
                        egui::Button::new(i18n.t("tuning.reset_defaults")),
                    )
                    .clicked()
                {
                    cfg.tuning.pl1_watts = None;
                    cfg.tuning.pl2_watts = None;
                    on_save();
                    on_cmd(WorkerCommand::RestoreDefaultPowerLimits {
                        keep_pl1: None,
                        keep_pl2: None,
                    });
                }
            });
        });
    });

    ui.add_space(8.0);

    // Card 2: CPU Fan Control + Interactive CPU Fan Curve
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("fan.cpu_section_title"), t);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if feature_toggle_button(
                    ui,
                    &i18n.t("fan.link_cpu_gpu"),
                    &mut cfg.thermal_fan.link_cpu_gpu_fans,
                    &i18n.t("common.enabled"),
                    t,
                )
                .on_hover_text(i18n.t("fan.link_cpu_gpu_desc"))
                .changed()
                {
                    if cfg.thermal_fan.link_cpu_gpu_fans {
                        cfg.thermal_fan.gpu_manual_speed_pct = cfg.thermal_fan.manual_speed_pct;
                    }
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            });
        });
        ui.add_space(4.0);

        render_fan_mode_selector(ui, cfg, effective_fan, i18n, t, on_cmd, on_save);
        ui.add_space(6.0);

        if cfg.thermal_fan.fan_mode == FanMode::Manual {
            ui.horizontal_wrapped(|ui| {
                ui.label(if cfg.thermal_fan.link_cpu_gpu_fans {
                    i18n.t("fan.manual_speed")
                } else {
                    i18n.t("fan.cpu_manual_speed")
                });
                let resp =
                    ui.add(Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100).suffix("%"));
                if control_committed(ui, "perf_cpu_manual_fan", &resp) {
                    if cfg.thermal_fan.link_cpu_gpu_fans {
                        cfg.thermal_fan.gpu_manual_speed_pct = cfg.thermal_fan.manual_speed_pct;
                    }
                    on_save();
                    on_cmd(WorkerCommand::SetFanMode {
                        mode: FanMode::Manual,
                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                        gpu_manual_speed_pct: Some(if cfg.thermal_fan.link_cpu_gpu_fans {
                            cfg.thermal_fan.manual_speed_pct
                        } else {
                            cfg.thermal_fan.gpu_manual_speed_pct
                        }),
                    });
                }
            });
        }

        if cfg.thermal_fan.fan_mode == FanMode::CustomCurve {
            ui.add_space(4.0);
            if st.telemetry.cpu_temp_c.is_none() {
                ui.colored_label(
                    warn_c,
                    i18n.t_fmt(
                        "fan.no_cpu_sensor_notice",
                        &[("temp", &cfg.thermal_fan.guard_cpu_temp_c.to_string())],
                    ),
                );
                ui.add_space(4.0);
            }

            let live_temp = effective_fan_curve_temp_c(
                st.telemetry.cpu_temp_c,
                if cfg.thermal_fan.link_cpu_gpu_fans {
                    st.telemetry.gpu_temp_c
                } else {
                    None
                },
                st.telemetry.chassis_temp_c,
                st.telemetry.cpu_power_w,
            );
            let max_rpm = st.telemetry.fan_max_rpms.first().copied().unwrap_or(5800);

            if render_fan_curve_editor(
                ui,
                "cpu_fan_curve",
                &mut cfg.thermal_fan.custom_curve,
                live_temp,
                max_rpm,
                &mut draft.curve_import_input,
                &mut draft.curve_status_msg,
                i18n,
                t,
            ) {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
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
