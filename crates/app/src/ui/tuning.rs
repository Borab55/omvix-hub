use crate::theme::{
    accent_color, card_frame, feature_toggle_button, muted_text, pill_button, section_title,
    status_badge, status_err, status_ok, status_warn,
};
use crate::ui::control_committed;
use crate::ui::performance::{
    render_fan_curve_editor, render_fan_mode_selector, PerformanceDraftState,
};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, FanMode, GpuMode, I18n};

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
    let err_c = status_err(t);
    let muted_c = muted_text(t);
    let effective_gpu_fan = if st.active_app_profile.is_some() {
        st.effective_gpu_fan_mode
    } else if cfg.thermal_fan.link_cpu_gpu_fans {
        cfg.thermal_fan.fan_mode
    } else {
        cfg.thermal_fan.gpu_fan_mode
    };

    ui.horizontal_wrapped(|ui| {
        ui.heading(i18n.t("drawer.sub_gpu"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let gpu_rpm = st
                .telemetry
                .fan_rpms
                .get(1)
                .or_else(|| st.telemetry.fan_rpms.first())
                .map(|r| format!("{r} RPM"))
                .unwrap_or_else(|| "—".into());
            status_badge(ui, &format!("Fan: {gpu_rpm}"), accent);
            if st.telemetry.gpu_sleeping {
                status_badge(ui, &i18n.t("cockpit.gpu_sleep_short"), muted_c);
            } else {
                if let Some(w) = st.telemetry.gpu_power_w {
                    status_badge(ui, &format!("{w:.0} W"), ok_c);
                }
                if let Some(temp) = st.telemetry.gpu_temp_c {
                    status_badge(ui, &format!("{temp:.0}°C"), warn_c);
                }
            }
        });
    });
    ui.add_space(6.0);

    // Card 1: Live GPU Telemetry & Power Summary
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("gpu.status_title"), t);
            if let Some(ref name) = st.telemetry.gpu_name {
                ui.label(RichText::new(name).size(12.0).color(muted_c));
            }
        });
        ui.add_space(4.0);

        if st.telemetry.gpu_sleeping {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, &i18n.t("gpu.sleeping_badge"), ok_c);
                ui.label(RichText::new(i18n.t("gpu.sleeping_desc")).size(12.0));
            });
        } else {
            egui::Grid::new("gpu_live_status_grid")
                .num_columns(2)
                .spacing([24.0, 5.0])
                .show(ui, |ui| {
                    ui.label(i18n.t("monitor.gpu_temp"));
                    ui.label(
                        st.telemetry
                            .gpu_temp_c
                            .map(|v| format!("{v:.1} °C"))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("monitor.gpu_usage"));
                    ui.label(
                        st.telemetry
                            .gpu_usage_pct
                            .map(|v| format!("{v:.0}%"))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("monitor.gpu_power"));
                    let pwr_str = match (st.telemetry.gpu_power_w, st.telemetry.gpu_power_limit_w) {
                        (Some(w), Some(lim)) => format!("{w:.1} W / {lim:.0} W"),
                        (Some(w), None) => format!("{w:.1} W"),
                        _ => "—".into(),
                    };
                    ui.label(pwr_str);
                    ui.end_row();

                    ui.label(i18n.t("gpu.clocks"));
                    let clk_str = match (st.telemetry.gpu_clock_mhz, st.telemetry.gpu_mem_clock_mhz)
                    {
                        (Some(core), Some(mem)) => format!("{core} MHz  /  {mem} MHz VRAM"),
                        (Some(core), None) => format!("{core} MHz"),
                        _ => "—".into(),
                    };
                    ui.label(clk_str);
                    ui.end_row();
                });
        }
    });

    ui.add_space(8.0);

    // Card 2: Independent GPU Fan Control & GPU Fan Curve
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("fan.gpu_section_title"), t);
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
                        cfg.thermal_fan.gpu_fan_mode = cfg.thermal_fan.fan_mode;
                        cfg.thermal_fan.gpu_manual_speed_pct = cfg.thermal_fan.manual_speed_pct;
                    }
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    if cfg.thermal_fan.link_cpu_gpu_fans {
                        on_cmd(WorkerCommand::SetFanMode {
                            mode: cfg.thermal_fan.fan_mode,
                            gpu_mode: Some(cfg.thermal_fan.fan_mode),
                            manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                            gpu_manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                        });
                    }
                }
            });
        });
        ui.add_space(4.0);

        if cfg.thermal_fan.link_cpu_gpu_fans {
            ui.weak(i18n.t("fan.gpu_linked_notice"));
        } else {
            render_fan_mode_selector(ui, cfg, effective_gpu_fan, true, i18n, t, on_cmd, on_save);
            ui.add_space(6.0);

            if cfg.thermal_fan.gpu_fan_mode == FanMode::Auto
                && cfg.thermal_fan.fan_mode != FanMode::Auto
            {
                ui.weak(RichText::new(i18n.t("fan.mixed_auto_note")).size(11.0));
                ui.add_space(4.0);
            }

            if cfg.thermal_fan.gpu_fan_mode == FanMode::Manual {
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n.t("fan.gpu_manual_speed"));
                    let resp = ui.add(
                        Slider::new(&mut cfg.thermal_fan.gpu_manual_speed_pct, 0..=100).suffix("%"),
                    );
                    if control_committed(ui, "perf_gpu_manual_fan", &resp) {
                        on_save();
                        on_cmd(WorkerCommand::SetFanMode {
                            mode: cfg.thermal_fan.fan_mode,
                            gpu_mode: Some(FanMode::Manual),
                            manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                            gpu_manual_speed_pct: Some(cfg.thermal_fan.gpu_manual_speed_pct),
                        });
                    }
                });
            }

            if cfg.thermal_fan.gpu_fan_mode == FanMode::CustomCurve {
                ui.add_space(4.0);
                let live_gpu_temp = if st.telemetry.gpu_sleeping {
                    st.telemetry.chassis_temp_c
                } else {
                    st.telemetry.gpu_temp_c.or(st.telemetry.chassis_temp_c)
                };
                let max_rpm = st
                    .telemetry
                    .fan_max_rpms
                    .get(1)
                    .copied()
                    .or_else(|| st.telemetry.fan_max_rpms.first().copied())
                    .unwrap_or(5800);

                if render_fan_curve_editor(
                    ui,
                    "gpu_fan_curve",
                    &mut cfg.thermal_fan.gpu_custom_curve,
                    live_gpu_temp,
                    max_rpm,
                    &mut draft.gpu_curve_import_input,
                    &mut draft.gpu_curve_status_msg,
                    i18n,
                    t,
                ) {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            }
        }
    });

    ui.add_space(8.0);

    // Card 3: Graphics Mode (GPU MUX Switch)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("gpu.mux_switch"), t);
        ui.weak(RichText::new(i18n.t("gpu.mux_desc")).size(11.0));
        ui.add_space(4.0);

        let cur = st.gpu_mode.unwrap_or(GpuMode::Unknown);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (GpuMode::Hybrid, "gpu.mode_hybrid"),
                (GpuMode::Discrete, "gpu.mode_discrete"),
                (GpuMode::Optimus, "gpu.mode_optimus"),
            ] {
                let selected = draft.pending_gpu_mode.unwrap_or(cur) == mode;
                if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                    if cur == mode {
                        draft.pending_gpu_mode = None;
                    } else {
                        draft.pending_gpu_mode = Some(mode);
                    }
                }
            }
        });

        if let Some(pending) = draft.pending_gpu_mode {
            ui.add_space(6.0);
            let mode_lbl = match pending {
                GpuMode::Hybrid => i18n.t("gpu.mode_hybrid"),
                GpuMode::Discrete => i18n.t("gpu.mode_discrete"),
                GpuMode::Optimus => i18n.t("gpu.mode_optimus"),
                GpuMode::Unknown => format!("{pending:?}"),
            };
            ui.colored_label(
                warn_c,
                RichText::new(i18n.t_fmt("gpu.mux_confirm_prompt", &[("mode", &mode_lbl)]))
                    .strong(),
            );
            ui.colored_label(
                err_c,
                RichText::new(i18n.t("gpu.bitlocker_warning")).size(11.5),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if pill_button(ui, &i18n.t("gpu.confirm_apply"), true, t).clicked() {
                    cfg.tuning.preferred_gpu_mode = Some(pending);
                    draft.pending_gpu_mode = None;
                    draft.gpu_reboot_dismissed = false;
                    on_save();
                    on_cmd(WorkerCommand::SetGpuMode(pending));
                }
                if ui.button(i18n.t("common.cancel")).clicked() {
                    draft.pending_gpu_mode = None;
                }
            });
        } else {
            ui.add_space(4.0);
            ui.weak(RichText::new(i18n.t("gpu.reboot_required")).size(11.5));
        }

        if st.gpu_mux_reboot_pending && !draft.gpu_reboot_dismissed {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(accent, RichText::new(i18n.t("gpu.reboot_prompt")).strong());
                if pill_button(ui, &i18n.t("gpu.reboot_now"), true, t).clicked() {
                    on_cmd(WorkerCommand::RebootSystem);
                }
                if ui.button(i18n.t("gpu.reboot_later")).clicked() {
                    draft.gpu_reboot_dismissed = true;
                }
            });
        }

        if !st.security.is_admin {
            ui.colored_label(warn_c, i18n.t("tuning.admin_required_warning"));
        }
    });
}
