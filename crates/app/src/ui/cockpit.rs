use crate::theme::{
    accent_color, clickable_status_badge, clickable_warning_badge, compact_card,
    compact_toggle_pill, detail_dial_button, fixed_color_picker_button, footer_text_button,
    lighting_profile_chip, lighting_swatch_button, mode_tile_button, muted_text, pill_button,
    saved_pill_chip, section_title, section_toggle_button, status_badge, status_err, status_ok,
    status_warn, tile_button, undo_text_button, ColorButtonStyle, SavedChipAction, TileIcon,
};
use crate::ui::lighting_display::{apply_lighting_profile, is_lighting_profile_active};
use crate::ui::{control_committed, Drawer, ExtraSubTab, FansPowerSubTab};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Slider, Ui};
use omvix_core::{
    AppConfig, BatteryProfile, FanMode, GpuMode, I18n, KeyboardType, LightingProfile,
    PerformanceMode, RgbColor, SafeModeReason, UpdateUiStatus,
};
use std::time::Duration;

#[derive(Default)]
pub struct CockpitActions {
    pub toggle_drawer: Option<Drawer>,
    pub open_drawer_exact: Option<Drawer>,
    pub open_fans_power_subtab: Option<FansPowerSubTab>,
    pub open_extra_subtab: Option<ExtraSubTab>,
    pub open_profiles: bool,
    pub open_updates: bool,
    pub restart_admin: bool,
    pub hide_to_tray: bool,
    pub quit_app: bool,
}

fn perf_mode_name(i18n: &I18n, mode: PerformanceMode) -> String {
    match mode {
        PerformanceMode::Quiet => i18n.t("thermal.mode_quiet"),
        PerformanceMode::Balanced => i18n.t("thermal.mode_balanced"),
        PerformanceMode::Performance => i18n.t("thermal.mode_performance"),
    }
}

fn fan_mode_name(i18n: &I18n, mode: FanMode) -> String {
    match mode {
        FanMode::Auto => i18n.t("cockpit.fan_auto"),
        FanMode::Max => i18n.t("cockpit.fan_max"),
        FanMode::Manual => i18n.t("cockpit.fan_manual"),
        FanMode::CustomCurve => i18n.t("cockpit.fan_curve"),
    }
}

fn temp_color(temp_c: Option<f32>, t: &omvix_core::CustomThemeConfig) -> Color32 {
    match temp_c {
        Some(v) if v >= 88.0 => status_err(t),
        Some(v) if v >= 78.0 => status_warn(t),
        _ => muted_text(t),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    open_drawer: Option<Drawer>,
    fans_power_tab: FansPowerSubTab,
    extra_tab: ExtraSubTab,
    pane_width: f32,
    worker_dead: bool,
    config_warnings: &mut Vec<String>,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) -> CockpitActions {
    let mut actions = CockpitActions::default();
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);
    let ok_c = status_ok(t);
    let warn_c = status_warn(t);
    let err_c = status_err(t);
    let muted_c = muted_text(t);

    // Explicitly pin cockpit interior width to match `cockpit_frame` (Margin 12) + `compact_card` (Margin 10 + Stroke 1)
    let content_w = (pane_width - 24.0).max(240.0);
    let card_inner_w = (content_w - 22.0).max(200.0);
    ui.set_width(content_w);
    ui.set_max_width(content_w);
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);

    // 1. Minimal G-Helper-style Header: Brand + Model on left; ONLY issues/actions on right
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(i18n.t("app.title"))
                .size(16.0)
                .strong()
                .color(accent),
        );
        if let Some(model) = st
            .diagnostics
            .as_ref()
            .map(|d| d.identity.model.trim())
            .filter(|m| !m.is_empty())
        {
            let short_model = if model.chars().count() > 26 {
                format!("— {}...", model.chars().take(23).collect::<String>())
            } else {
                format!("— {model}")
            };
            ui.label(RichText::new(short_model).size(12.0).color(muted_c));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !st.security.is_admin && clickable_status_badge(ui, "Admin ?", warn_c).clicked() {
                actions.restart_admin = true;
            }

            let running_conflicts = st
                .conflict_services
                .iter()
                .filter(|s| s.status.eq_ignore_ascii_case("Running"))
                .count();
            if running_conflicts > 0
                && clickable_warning_badge(ui, running_conflicts, warn_c)
                    .on_hover_text(i18n.t_fmt(
                        "conflicts.detected_summary",
                        &[("count", &running_conflicts.to_string())],
                    ))
                    .clicked()
            {
                actions.open_extra_subtab = Some(ExtraSubTab::Diagnostics);
            }

            if st.thermal_guard_active {
                status_badge(ui, "MAX FAN", err_c);
            }
            if st.sensor_loss_active {
                status_badge(ui, &i18n.t("cockpit.sensor_loss_badge"), err_c);
            }
            if st.safe_mode.is_some() {
                status_badge(ui, &i18n.t("safe_mode.badge"), warn_c);
            }
            let available_ver = match &st.update_status {
                UpdateUiStatus::UpdateAvailable(rel) => Some(rel.version.as_str()),
                UpdateUiStatus::ReadyToInstall(staged) => Some(staged.version.as_str()),
                _ => None,
            };
            if let Some(ver) = available_ver {
                if clickable_status_badge(ui, &format!("v{ver} ↑"), ok_c)
                    .on_hover_text(i18n.t("updates.cockpit_tooltip"))
                    .clicked()
                {
                    actions.open_updates = true;
                }
            }
        });
    });

    // Conditional Problem / Warning Banners (HIDDEN during normal healthy operation)
    if worker_dead {
        ui.add_space(3.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.colored_label(
                err_c,
                RichText::new(i18n.t("app.worker_dead_warning"))
                    .size(11.5)
                    .strong(),
            );
        });
    }

    if let Some(reason) = st.safe_mode {
        ui.add_space(3.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.colored_label(
                warn_c,
                RichText::new(i18n.t("safe_mode.banner_title"))
                    .size(11.5)
                    .strong(),
            );
            let reason_key = match reason {
                SafeModeReason::CliFlag => "safe_mode.reason_cli",
                SafeModeReason::ShiftHeld => "safe_mode.reason_shift",
                SafeModeReason::CrashLoop => "safe_mode.reason_crash_loop",
            };
            ui.weak(RichText::new(i18n.t(reason_key)).size(10.5));
            ui.horizontal_wrapped(|ui| {
                if pill_button(ui, &i18n.t("safe_mode.exit_btn"), false, t).clicked() {
                    on_cmd(WorkerCommand::ExitSafeMode);
                }
                if ui.small_button(i18n.t("safe_mode.open_settings")).clicked() {
                    actions.open_extra_subtab = Some(ExtraSubTab::Settings);
                }
            });
        });
    }

    if !config_warnings.is_empty() {
        ui.add_space(3.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2((card_inner_w - 28.0).max(120.0), 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(config_warnings.join(" | "))
                                    .size(11.0)
                                    .color(warn_c),
                            )
                            .wrap(),
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui.small_button("×").clicked() {
                        config_warnings.clear();
                    }
                });
            });
        });
    }

    if !st.board_known && !st.bios_writes_allowed && !st.board_id.is_empty() {
        ui.add_space(3.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.colored_label(
                warn_c,
                RichText::new(i18n.t_fmt("board.experimental_title", &[("board", &st.board_id)]))
                    .size(11.5)
                    .strong(),
            );
            ui.weak(RichText::new(i18n.t("board.experimental_desc")).size(10.5));
            ui.horizontal_wrapped(|ui| {
                if pill_button(ui, &i18n.t("board.approve_experimental"), false, t).clicked() {
                    cfg.general.experimental_board_id = Some(st.board_id.clone());
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
                if ui.small_button(i18n.t("board.view_probes")).clicked() {
                    actions.open_extra_subtab = Some(ExtraSubTab::Diagnostics);
                }
            });
        });
    }

    if !st.security.is_admin {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.colored_label(
                warn_c,
                RichText::new(i18n.t("app.restart_as_admin")).size(11.5),
            );
            ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
            if ui.small_button(i18n.t("common.apply")).clicked() {
                actions.restart_admin = true;
            }
        });
    }

    ui.add_space(5.0);

    // 2. Clean G-Helper-style Main Cockpit Sections
    let avail_h = ui.available_height();
    let reserve_footer = 66.0_f32;
    egui::ScrollArea::vertical()
        .id_salt("cockpit_middle_scroll")
        .max_width(content_w)
        .max_height((avail_h - reserve_footer).max(200.0))
        .show(ui, |ui| {
            ui.set_width(content_w);
            ui.set_max_width(content_w);

            let effective_perf = if st.active_app_profile.is_some() {
                st.effective_performance_mode
            } else {
                cfg.thermal_fan.performance_mode
            };
            let effective_cpu_fan = if st.active_app_profile.is_some() {
                st.effective_fan_mode
            } else {
                cfg.thermal_fan.fan_mode
            };
            let effective_gpu_fan = if st.active_app_profile.is_some() {
                st.effective_gpu_fan_mode
            } else if cfg.thermal_fan.link_cpu_gpu_fans {
                cfg.thermal_fan.fan_mode
            } else {
                cfg.thermal_fan.gpu_fan_mode
            };

            // Card A: Performance Mode (`Mod: <Mode> · 25W`  |  `CPU: 37°C  Fan: 2000 RPM`)
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);

                ui.horizontal(|ui| {
                    let mode_title = format!(
                        "{}: {}",
                        i18n.t("cockpit.mode_prefix"),
                        perf_mode_name(i18n, effective_perf)
                    );
                    section_title(ui, mode_title, t);
                    if let Some(cpu_w) = st.telemetry.cpu_power_w {
                        ui.label(
                            RichText::new(format!("{cpu_w:.0}W"))
                                .size(11.5)
                                .color(muted_c),
                        );
                    }
                    if let Some(ref prof_name) = st.active_app_profile {
                        let short_name = if prof_name.chars().count() > 11 {
                            format!("{}...", prof_name.chars().take(9).collect::<String>())
                        } else {
                            prof_name.clone()
                        };
                        if clickable_status_badge(ui, &short_name, ok_c).clicked() {
                            actions.open_profiles = true;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let cpu_open = open_drawer == Some(Drawer::FansPower)
                            && fans_power_tab == FansPowerSubTab::Cpu;
                        let cpu_tip = i18n.t_fmt(
                            "cockpit.open_detail_tooltip",
                            &[("section", &i18n.t("drawer.sub_cpu"))],
                        );
                        if detail_dial_button(ui, cpu_open, &cpu_tip, t).clicked() {
                            actions.open_fans_power_subtab = Some(FansPowerSubTab::Cpu);
                        }
                        if compact_toggle_pill(
                            ui,
                            &i18n.t("profiles.cockpit_auto_badge"),
                            cfg.auto_profile.enabled,
                            t,
                        )
                        .on_hover_text(i18n.t("cockpit.auto_acdc_tooltip"))
                        .clicked()
                        {
                            cfg.auto_profile.enabled = !cfg.auto_profile.enabled;
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            if cfg.auto_profile.enabled {
                                on_cmd(WorkerCommand::ApplyAutoProfileNow);
                            }
                        }

                        let cpu_temp_str = st
                            .telemetry
                            .cpu_temp_c
                            .map(|v| format!("{v:.0}°C"))
                            .unwrap_or_else(|| "—".into());
                        let cpu_rpm_str = st
                            .telemetry
                            .fan_rpms
                            .first()
                            .map(|r| format!("{r} RPM"))
                            .unwrap_or_else(|| "—".into());
                        let stat_txt = format!("CPU: {cpu_temp_str}  Fan: {cpu_rpm_str}");
                        ui.label(
                            RichText::new(stat_txt)
                                .size(11.5)
                                .color(temp_color(st.telemetry.cpu_temp_c, t)),
                        );
                    });
                });

                ui.add_space(4.0);
                let items = [
                    (
                        PerformanceMode::Quiet,
                        TileIcon::Feather,
                        i18n.t("thermal.mode_quiet"),
                        Color32::from_rgb(56, 189, 248),
                    ),
                    (
                        PerformanceMode::Balanced,
                        TileIcon::Gauge,
                        i18n.t("thermal.mode_balanced"),
                        accent,
                    ),
                    (
                        PerformanceMode::Performance,
                        TileIcon::Rocket,
                        i18n.t("thermal.mode_performance"),
                        Color32::from_rgb(239, 68, 68),
                    ),
                ];
                let w = equal_tile_width(card_inner_w, items.len(), 6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (mode, icon, label, border_c) in items {
                        let selected = effective_perf == mode;
                        if mode_tile_button(ui, icon, &label, selected, Some(border_c), w, 46.0, t)
                            .clicked()
                        {
                            cfg.thermal_fan.performance_mode = mode;
                            on_save();
                            on_cmd(WorkerCommand::SetPerformanceMode(mode));
                        }
                    }
                });
            });

            ui.add_space(5.0);

            // Card B: Fan & GPU (`Fan: <Mode>`  |  `GPU: 40°C  Fan: 2300 RPM`)
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);

                ui.horizontal(|ui| {
                    let fan_title = if effective_cpu_fan == effective_gpu_fan {
                        format!(
                            "{}: {}",
                            i18n.t("cockpit.fan_prefix"),
                            fan_mode_name(i18n, effective_cpu_fan)
                        )
                    } else {
                        format!(
                            "{}: CPU {} / GPU {}",
                            i18n.t("cockpit.fan_prefix"),
                            fan_mode_name(i18n, effective_cpu_fan),
                            fan_mode_name(i18n, effective_gpu_fan)
                        )
                    };
                    section_title(ui, fan_title, t);
                    if let Some(gpu_m) = st.gpu_mode.filter(|&m| m != GpuMode::Unknown) {
                        let mux_lbl = match gpu_m {
                            GpuMode::Hybrid => i18n.t("tuning.gpu_hybrid_short"),
                            GpuMode::Discrete => i18n.t("tuning.gpu_discrete_short"),
                            GpuMode::Optimus => i18n.t("tuning.gpu_optimus_short"),
                            GpuMode::Unknown => String::new(),
                        };
                        let mux_clr = if gpu_m == GpuMode::Discrete {
                            warn_c
                        } else {
                            muted_c
                        };
                        if clickable_status_badge(ui, &mux_lbl, mux_clr).clicked() {
                            actions.open_fans_power_subtab = Some(FansPowerSubTab::Gpu);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let gpu_open = open_drawer == Some(Drawer::FansPower)
                            && fans_power_tab == FansPowerSubTab::Gpu;
                        let gpu_tip = i18n.t_fmt(
                            "cockpit.open_detail_tooltip",
                            &[("section", &i18n.t("drawer.sub_gpu"))],
                        );
                        if detail_dial_button(ui, gpu_open, &gpu_tip, t).clicked() {
                            actions.open_fans_power_subtab = Some(FansPowerSubTab::Gpu);
                        }

                        let gpu_rpm = st
                            .telemetry
                            .fan_rpms
                            .get(1)
                            .or_else(|| st.telemetry.fan_rpms.first())
                            .map(|r| format!("{r} RPM"))
                            .unwrap_or_else(|| "—".into());
                        if st.telemetry.gpu_sleeping {
                            let sleep_txt = format!(
                                "GPU: {}  Fan: {gpu_rpm}",
                                i18n.t("cockpit.gpu_sleep_short")
                            );
                            ui.label(
                                RichText::new(sleep_txt)
                                    .size(11.5)
                                    .color(Color32::from_rgb(138, 162, 194)),
                            )
                            .on_hover_text(i18n.t("monitor.gpu_sleeping_tooltip"));
                        } else {
                            let gpu_t = st
                                .telemetry
                                .gpu_temp_c
                                .map(|v| format!("{v:.0}°C"))
                                .unwrap_or_else(|| "—".into());
                            ui.label(
                                RichText::new(format!("GPU: {gpu_t}  Fan: {gpu_rpm}"))
                                    .size(11.5)
                                    .color(temp_color(st.telemetry.gpu_temp_c, t)),
                            );
                        }
                    });
                });

                ui.add_space(4.0);
                let items = [
                    (
                        FanMode::Auto,
                        TileIcon::FanAuto,
                        i18n.t("cockpit.fan_auto"),
                        accent,
                    ),
                    (
                        FanMode::CustomCurve,
                        TileIcon::FanCurve,
                        i18n.t("cockpit.fan_curve"),
                        accent,
                    ),
                    (
                        FanMode::Manual,
                        TileIcon::FanSliders,
                        i18n.t("cockpit.fan_manual"),
                        accent,
                    ),
                    (
                        FanMode::Max,
                        TileIcon::FanMax,
                        i18n.t("cockpit.fan_max"),
                        Color32::from_rgb(245, 158, 11),
                    ),
                ];
                let w = equal_tile_width(card_inner_w, items.len(), 6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (mode, icon, label, border_c) in items {
                        let selected = effective_cpu_fan == mode && effective_gpu_fan == mode;
                        if mode_tile_button(ui, icon, &label, selected, Some(border_c), w, 46.0, t)
                            .clicked()
                        {
                            cfg.thermal_fan.fan_mode = mode;
                            cfg.thermal_fan.gpu_fan_mode = mode;
                            on_save();
                            let cpu_pct = cfg.thermal_fan.manual_speed_pct;
                            let gpu_pct = if cfg.thermal_fan.link_cpu_gpu_fans {
                                cpu_pct
                            } else {
                                cfg.thermal_fan.gpu_manual_speed_pct
                            };
                            on_cmd(WorkerCommand::SetFanMode {
                                mode,
                                gpu_mode: Some(mode),
                                manual_speed_pct: Some(cpu_pct),
                                gpu_manual_speed_pct: Some(gpu_pct),
                            });
                        }
                    }
                });

                if effective_cpu_fan == FanMode::Manual || effective_gpu_fan == FanMode::Manual {
                    ui.add_space(4.0);
                    if cfg.thermal_fan.link_cpu_gpu_fans && effective_cpu_fan == FanMode::Manual {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(i18n.t("fan.manual_speed")).size(12.0));
                            ui.spacing_mut().slider_width = (card_inner_w - 135.0).max(160.0);
                            let resp = ui.add(
                                Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100)
                                    .suffix("%"),
                            );
                            if control_committed(ui, "cockpit_manual_linked_fan", &resp) {
                                cfg.thermal_fan.gpu_manual_speed_pct =
                                    cfg.thermal_fan.manual_speed_pct;
                                on_save();
                                on_cmd(WorkerCommand::SetFanMode {
                                    mode: FanMode::Manual,
                                    gpu_mode: Some(FanMode::Manual),
                                    manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                                    gpu_manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                                });
                            }
                        });
                    } else {
                        if effective_cpu_fan == FanMode::Manual {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(i18n.t("fan.cpu_fan_short")).size(12.0));
                                ui.spacing_mut().slider_width = (card_inner_w - 125.0).max(160.0);
                                let resp = ui.add(
                                    Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100)
                                        .suffix("%"),
                                );
                                if control_committed(ui, "cockpit_manual_cpu_fan", &resp) {
                                    on_save();
                                    on_cmd(WorkerCommand::SetFanMode {
                                        mode: FanMode::Manual,
                                        gpu_mode: Some(cfg.thermal_fan.gpu_fan_mode),
                                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                                        gpu_manual_speed_pct: Some(
                                            cfg.thermal_fan.gpu_manual_speed_pct,
                                        ),
                                    });
                                }
                            });
                        }
                        if effective_gpu_fan == FanMode::Manual {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(i18n.t("fan.gpu_fan_short")).size(12.0));
                                ui.spacing_mut().slider_width = (card_inner_w - 125.0).max(160.0);
                                let resp = ui.add(
                                    Slider::new(&mut cfg.thermal_fan.gpu_manual_speed_pct, 0..=100)
                                        .suffix("%"),
                                );
                                if control_committed(ui, "cockpit_manual_gpu_fan", &resp) {
                                    on_save();
                                    on_cmd(WorkerCommand::SetFanMode {
                                        mode: cfg.thermal_fan.fan_mode,
                                        gpu_mode: Some(FanMode::Manual),
                                        manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                                        gpu_manual_speed_pct: Some(
                                            cfg.thermal_fan.gpu_manual_speed_pct,
                                        ),
                                    });
                                }
                            });
                        }
                    }
                }
            });

            ui.add_space(5.0);

            // Card C: Laptop Screen Refresh Rate
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                ui.horizontal(|ui| {
                    let disp_title = match &st.display_mode {
                        Some(d) => format!("{}: {} Hz", i18n.t("home.quick_display"), d.current_hz),
                        None => i18n.t("home.quick_display"),
                    };
                    section_title(ui, disp_title, t);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let disp_open = open_drawer == Some(Drawer::Extra)
                            && extra_tab == ExtraSubTab::LightingDisplay;
                        let disp_tip = i18n.t_fmt(
                            "cockpit.open_detail_tooltip",
                            &[("section", &i18n.t("drawer.sub_lighting_display"))],
                        );
                        if detail_dial_button(ui, disp_open, &disp_tip, t).clicked() {
                            actions.open_extra_subtab = Some(ExtraSubTab::LightingDisplay);
                        }
                        if let Some(disp) = &st.display_mode {
                            ui.label(
                                RichText::new(format!("{}x{}", disp.width, disp.height))
                                    .size(11.0)
                                    .color(muted_c),
                            );
                        }
                    });
                });
                ui.add_space(3.0);
                if let Some(disp) = &st.display_mode {
                    if !disp.supported_hz.is_empty() {
                        let compact_hz = disp.supported_hz.len() >= 5;
                        let w = equal_tile_width(card_inner_w, disp.supported_hz.len(), 5.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            for &hz in &disp.supported_hz {
                                let label = if compact_hz {
                                    format!("{hz}Hz")
                                } else {
                                    i18n.t_fmt("display.hz", &[("hz", &hz.to_string())])
                                };
                                if tile_button(ui, &label, disp.current_hz == hz, w, 25.0, t)
                                    .clicked()
                                {
                                    on_cmd(WorkerCommand::SetRefreshRate(hz));
                                }
                            }
                        });
                    }
                } else {
                    ui.weak("...");
                }
            });

            ui.add_space(5.0);

            // Card D: Battery Charge Limit (clean 1-row limits + live % on right)
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                let now_utc = chrono::Utc::now().timestamp();
                let travel_active = cfg.battery.is_travel_charge_active(now_utc);

                ui.horizontal(|ui| {
                    let bat_title = if cfg.battery.enabled && !travel_active {
                        format!(
                            "{}: {}%",
                            i18n.t("home.quick_battery"),
                            cfg.battery.stop_threshold_pct
                        )
                    } else if travel_active {
                        format!("{}: 1x 100%", i18n.t("home.quick_battery"))
                    } else {
                        format!("{}: 100%", i18n.t("home.quick_battery"))
                    };
                    section_title(ui, bat_title, t);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let bat_open = open_drawer == Some(Drawer::Battery);
                        let bat_tip = i18n.t_fmt(
                            "cockpit.open_detail_tooltip",
                            &[("section", &i18n.t("nav.battery"))],
                        );
                        if detail_dial_button(ui, bat_open, &bat_tip, t).clicked() {
                            actions.open_drawer_exact = Some(Drawer::Battery);
                        }
                        if let Some(pct) = st.telemetry.battery_pct {
                            let src = match st.telemetry.ac_connected {
                                Some(true) => i18n.t("dashboard.power_source_ac"),
                                Some(false) => i18n.t("dashboard.power_source_dc"),
                                None => String::new(),
                            };
                            let bat_live = if src.is_empty() {
                                format!("{pct}%")
                            } else {
                                format!("{pct}% · {src}")
                            };
                            ui.label(RichText::new(bat_live).size(11.5).color(muted_c));
                        }
                    });
                });

                ui.add_space(3.0);
                let builtins = BatteryProfile::builtins();
                let pw = equal_tile_width(card_inner_w, builtins.len() + 1, 5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    for preset in builtins {
                        let short_label = match preset.id.as_str() {
                            "desktop" => "60%",
                            "balanced" => "80%",
                            "travel" => "90%",
                            "full" => "100%",
                            _ => &preset.name,
                        };
                        let selected = is_profile_active(cfg, &preset) && !travel_active;
                        if tile_button(ui, short_label, selected, pw, 25.0, t).clicked() {
                            cfg.battery.clear_travel_charge();
                            apply_battery_profile(cfg, &preset);
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyBatteryNow);
                        }
                    }
                    let t_label = i18n.t("cockpit.travel_short");
                    if tile_button(ui, &t_label, travel_active, pw, 25.0, t)
                        .on_hover_text(i18n.t("cockpit.travel_short_tooltip"))
                        .clicked()
                    {
                        if travel_active {
                            cfg.battery.clear_travel_charge();
                        } else {
                            cfg.battery.enable_travel_charge(now_utc);
                        }
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                });

                if !cfg.battery.custom_profiles.is_empty() {
                    ui.add_space(3.0);
                    let del_tip = i18n.t("battery.delete");
                    let mut apply_custom = None;
                    let mut delete_idx: Option<usize> = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
                        for (idx, custom) in cfg.battery.custom_profiles.iter().enumerate() {
                            let selected =
                                cfg.battery.active_profile_id.as_deref() == Some(&custom.id);
                            let label = format!("{} ({}%)", custom.name, custom.stop_threshold_pct);
                            let tip = format!(
                                "{}% / {}%  •  {:?}",
                                custom.stop_threshold_pct, custom.start_threshold_pct, custom.mode
                            );
                            match saved_pill_chip(
                                ui,
                                ("cockpit_bat", &custom.id, idx),
                                &label,
                                selected,
                                Some(&tip),
                                &del_tip,
                                t,
                            ) {
                                SavedChipAction::Apply => apply_custom = Some(custom.clone()),
                                SavedChipAction::Delete => delete_idx = Some(idx),
                                SavedChipAction::None => {}
                            }
                        }
                    });
                    if let Some(custom) = apply_custom {
                        apply_battery_profile(cfg, &custom);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                    if let Some(idx) = delete_idx {
                        let removed = cfg.battery.custom_profiles.remove(idx);
                        if cfg.battery.active_profile_id.as_deref() == Some(&removed.id) {
                            cfg.battery.active_profile_id = None;
                        }
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                }
            });

            ui.add_space(5.0);

            // Card E: Keyboard Lighting
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                ui.horizontal(|ui| {
                    if section_toggle_button(
                        ui,
                        &i18n.t("home.quick_keyboard"),
                        &mut cfg.lighting.enabled,
                        &i18n.t("common.enabled"),
                        t,
                    )
                    .changed()
                    {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let light_open = open_drawer == Some(Drawer::Extra)
                            && extra_tab == ExtraSubTab::LightingDisplay;
                        let light_tip = i18n.t_fmt(
                            "cockpit.open_detail_tooltip",
                            &[("section", &i18n.t("drawer.sub_lighting_display"))],
                        );
                        if detail_dial_button(ui, light_open, &light_tip, t).clicked() {
                            actions.open_extra_subtab = Some(ExtraSubTab::LightingDisplay);
                        }

                        let c0 = &mut cfg.lighting.zones[0];
                        let mut srgb = [c0.r, c0.g, c0.b];
                        let resp = fixed_color_picker_button(
                            ui,
                            "cockpit_kbd_wheel_picker",
                            &mut srgb,
                            ColorButtonStyle::ColorWheel { size: 22.0 },
                            t,
                        );
                        if resp.changed() {
                            let picked = RgbColor {
                                r: srgb[0],
                                g: srgb[1],
                                b: srgb[2],
                            };
                            cfg.lighting.zones = [picked; 4];
                            cfg.lighting.enabled = true;
                            cfg.lighting.active_profile_id = None;
                        }
                        if control_committed(ui, "cockpit_kbd_color", &resp) {
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyLighting);
                        }
                    });
                });

                ui.add_space(3.0);
                let four_zone = st.keyboard_type == KeyboardType::FourZoneRgb;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
                    for preset in LightingProfile::builtins() {
                        let selected = is_lighting_profile_active(cfg, &preset);
                        if lighting_swatch_button(ui, &preset.zones, selected, &preset.name, t)
                            .clicked()
                        {
                            apply_lighting_profile(cfg, &preset);
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyLighting);
                        }
                    }
                    if !cfg.lighting.custom_profiles.is_empty() {
                        let del_tip = i18n.t("lighting.delete_profile");
                        let mut apply_prof = None;
                        let mut delete_idx: Option<usize> = None;
                        for (idx, custom) in cfg.lighting.custom_profiles.iter().enumerate() {
                            let selected = is_lighting_profile_active(cfg, custom);
                            match lighting_profile_chip(
                                ui,
                                ("cockpit_kbd", &custom.id, idx),
                                &custom.name,
                                &custom.zones,
                                selected,
                                four_zone,
                                &del_tip,
                                t,
                            ) {
                                SavedChipAction::Apply => apply_prof = Some(custom.clone()),
                                SavedChipAction::Delete => delete_idx = Some(idx),
                                SavedChipAction::None => {}
                            }
                        }
                        if let Some(custom) = apply_prof {
                            apply_lighting_profile(cfg, &custom);
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyLighting);
                        }
                        if let Some(idx) = delete_idx {
                            let removed = cfg.lighting.custom_profiles.remove(idx);
                            if cfg.lighting.active_profile_id.as_deref() == Some(&removed.id) {
                                cfg.lighting.active_profile_id = None;
                            }
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        }
                    }
                });

                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(i18n.t("lighting.brightness"));
                    ui.spacing_mut().slider_width = (card_inner_w - 135.0).max(185.0);
                    let resp =
                        ui.add(Slider::new(&mut cfg.lighting.brightness_pct, 0..=100).suffix("%"));
                    if resp.changed() {
                        cfg.lighting.active_profile_id = None;
                    }
                    if control_committed(ui, "cockpit_kbd_brightness", &resp) {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                });
            });
        });

    // 3. Bottom Drawer Navigation & Compact Footer
    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.set_width(content_w);
        ui.set_max_width(content_w);

        const FOOTER_FONT_SIZE: f32 = 12.5;
        ui.horizontal(|ui| {
            let ver_label = format!("v{}", env!("CARGO_PKG_VERSION"));
            if footer_text_button(ui, &ver_label, FOOTER_FONT_SIZE, muted_c, accent)
                .on_hover_text(i18n.t("cockpit.version_tooltip"))
                .clicked()
            {
                actions.open_updates = true;
            }
            if let Some((msg, is_err, ts)) = &st.last_toast {
                if ts.elapsed() < Duration::from_secs(6) {
                    let color = if *is_err { err_c } else { ok_c };
                    let short = if msg.chars().count() > 26 {
                        format!("{}...", msg.chars().take(23).collect::<String>())
                    } else {
                        msg.clone()
                    };
                    ui.colored_label(color, RichText::new(short).size(11.0));
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if footer_text_button(
                    ui,
                    &i18n.t("cockpit.quit"),
                    FOOTER_FONT_SIZE,
                    muted_c,
                    err_c,
                )
                .clicked()
                {
                    actions.quit_app = true;
                }
                if footer_text_button(
                    ui,
                    &i18n.t("cockpit.hide_to_tray"),
                    FOOTER_FONT_SIZE,
                    muted_c,
                    accent,
                )
                .clicked()
                {
                    actions.hide_to_tray = true;
                }
                if let Some(latest) = st.change_history.first() {
                    let domain_lbl = i18n.t(latest.domain.i18n_key());
                    let tip = i18n.t_fmt(
                        "history.undo_latest_tooltip",
                        &[
                            ("domain", &domain_lbl),
                            ("old", &latest.old_summary),
                            ("new", &latest.new_summary),
                        ],
                    );
                    if undo_text_button(
                        ui,
                        &i18n.t("history.undo_short"),
                        FOOTER_FONT_SIZE,
                        muted_c,
                        t,
                    )
                    .on_hover_text(tip)
                    .clicked()
                    {
                        on_cmd(WorkerCommand::UndoLastChange);
                    }
                }
            });
        });

        ui.add_space(6.0);

        let drawers = [
            (Drawer::Monitor, i18n.t("drawer.monitor")),
            (Drawer::FansPower, i18n.t("drawer.fans_power")),
            (Drawer::Battery, i18n.t("drawer.battery")),
            (Drawer::Extra, i18n.t("drawer.extra")),
        ];
        let dw = equal_tile_width(content_w, drawers.len(), 5.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            for (d, label) in drawers {
                let selected = open_drawer == Some(d);
                if tile_button(ui, &label, selected, dw, 28.0, t).clicked() {
                    actions.toggle_drawer = Some(d);
                }
            }
        });
        ui.add_space(2.0);
    });

    actions
}

fn equal_tile_width(avail: f32, count: usize, spacing: f32) -> f32 {
    let n = count.max(1) as f32;
    ((avail - spacing * (n - 1.0)) / n).floor().max(36.0)
}

pub fn apply_battery_profile(cfg: &mut AppConfig, profile: &BatteryProfile) {
    cfg.battery.enabled = profile.enabled;
    cfg.battery.stop_threshold_pct = profile.stop_threshold_pct;
    cfg.battery.start_threshold_pct = profile.start_threshold_pct;
    cfg.battery.mode = profile.mode;
    cfg.battery.active_profile_id = Some(profile.id.clone());
}

pub fn is_profile_active(cfg: &AppConfig, profile: &BatteryProfile) -> bool {
    if cfg.battery.active_profile_id.as_deref() == Some(&profile.id) {
        return true;
    }
    cfg.battery.enabled == profile.enabled
        && cfg.battery.stop_threshold_pct == profile.stop_threshold_pct
        && cfg.battery.start_threshold_pct == profile.start_threshold_pct
        && cfg.battery.mode == profile.mode
}
