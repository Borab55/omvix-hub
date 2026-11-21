use crate::theme::{
    STATUS_ERR, STATUS_OK, STATUS_WARN, accent_color, clickable_status_badge,
    clickable_warning_badge, compact_card, pill_button, rgb, section_title, status_badge,
    tile_button,
};
use crate::ui::{Drawer, ExtraSubTab, control_committed};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, CornerRadius, RichText, Slider, Stroke, Ui};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryProfile, CpuBoostMode, FanMode, I18n, PerformanceMode,
    RgbColor,
};
use std::time::Duration;

#[derive(Default)]
pub struct CockpitActions {
    pub toggle_drawer: Option<Drawer>,
    pub open_extra_subtab: Option<ExtraSubTab>,
    pub restart_admin: bool,
    pub hide_to_tray: bool,
    pub quit_app: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    open_drawer: Option<Drawer>,
    pane_width: f32,
    worker_dead: bool,
    config_warnings: &mut Vec<String>,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) -> CockpitActions {
    let mut actions = CockpitActions::default();
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    // Explicitly pin cockpit interior width to match `cockpit_frame` (Margin 12) + `compact_card` (Margin 10 + Stroke 1)
    let content_w = (pane_width - 24.0).max(240.0);
    let card_inner_w = (content_w - 22.0).max(200.0);
    ui.set_width(content_w);
    ui.set_max_width(content_w);
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);

    // 1. Header Row: Brand + Detected Model + Admin / Conflict badges
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(i18n.t("app.title"))
                    .size(18.0)
                    .strong()
                    .color(accent),
            );
            let device_sub = st
                .diagnostics
                .as_ref()
                .map(|d| d.identity.model.trim())
                .filter(|m| !m.is_empty())
                .map(|m| m.to_string())
                .unwrap_or_else(|| i18n.t("app.subtitle"));
            ui.weak(RichText::new(device_sub).size(11.0));
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !st.security.is_admin
                && clickable_status_badge(ui, "Admin ?", STATUS_WARN).clicked()
            {
                actions.restart_admin = true;
            }

            let running_conflicts = st
                .conflict_services
                .iter()
                .filter(|s| s.status.eq_ignore_ascii_case("Running"))
                .count();
            if running_conflicts > 0
                && clickable_warning_badge(ui, running_conflicts, STATUS_WARN)
                    .on_hover_text(i18n.t_fmt(
                        "conflicts.detected_summary",
                        &[("count", &running_conflicts.to_string())],
                    ))
                    .clicked()
            {
                actions.open_extra_subtab = Some(ExtraSubTab::Diagnostics);
            }

            if st.thermal_guard_active {
                status_badge(ui, "MAX GUARD", STATUS_ERR);
            }
            if st.sensor_loss_active {
                status_badge(ui, &i18n.t("cockpit.sensor_loss_badge"), STATUS_ERR);
            }
        });
    });

    ui.add_space(4.0);

    // 2. Glanceable Live Telemetry Strip
    compact_card(t).show(ui, |ui| {
        ui.set_width(card_inner_w);
        ui.set_max_width(card_inner_w);

        let cpu_u = st
            .telemetry
            .cpu_usage_pct
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "—".into());
        let cpu_w = st
            .telemetry
            .cpu_power_w
            .map(|v| format!("{v:.0}W"))
            .unwrap_or_else(|| "—".into());
        let cpu_line = match st.telemetry.cpu_temp_c {
            Some(v) => format!("CPU {cpu_u} · {v:.0}°C · {cpu_w}"),
            None => format!("CPU {cpu_u} · {cpu_w}"),
        };

        let gpu_u = st
            .telemetry
            .gpu_usage_pct
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "—".into());
        let gpu_t = st
            .telemetry
            .gpu_temp_c
            .map(|v| format!("{v:.0}°C"))
            .unwrap_or_else(|| "—".into());
        let gpu_w = st
            .telemetry
            .gpu_power_w
            .map(|v| format!("{v:.0}W"))
            .unwrap_or_else(|| "—".into());

        ui.horizontal(|ui| {
            ui.label(RichText::new(cpu_line).size(12.5).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("GPU {gpu_u} · {gpu_t} · {gpu_w}"))
                        .size(12.5)
                        .strong(),
                );
            });
        });

        let fans = if st.telemetry.fan_rpms.is_empty() {
            "Fan: —".to_string()
        } else {
            st.telemetry
                .fan_rpms
                .iter()
                .enumerate()
                .map(|(i, r)| format!("F{}: {r}", i + 1))
                .collect::<Vec<_>>()
                .join(" · ")
                + " RPM"
        };
        let fans_and_chassis = match st.telemetry.chassis_temp_c {
            Some(ch) if st.telemetry.cpu_temp_c.is_none() => {
                format!("{fans} · {} {ch:.0}°C", i18n.t("cockpit.chassis_short"))
            }
            _ => fans,
        };
        let ram = match (st.telemetry.ram_used_gb, st.telemetry.ram_total_gb) {
            (Some(u), Some(tot)) if tot > 0.0 => {
                let pct = ((u / tot) * 100.0).clamp(0.0, 100.0);
                format!("RAM {u:.1}/{tot:.0}G ({pct:.0}%)")
            }
            (Some(u), Some(tot)) => format!("RAM {u:.1}/{tot:.0}G"),
            _ => "RAM —".into(),
        };
        let bat = st
            .telemetry
            .battery_pct
            .map(|p| {
                let src = match st.telemetry.ac_connected {
                    Some(true) => "AC",
                    Some(false) => "DC",
                    None => "",
                };
                if src.is_empty() {
                    format!("Bat {p}%")
                } else {
                    format!("Bat {p}% {src}")
                }
            })
            .unwrap_or_else(|| "Bat —".into());

        ui.horizontal(|ui| {
            ui.weak(RichText::new(fans_and_chassis).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(RichText::new(format!("{ram}  |  {bat}")).size(11.0));
            });
        });
    });

    if worker_dead {
        ui.add_space(4.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.colored_label(
                STATUS_ERR,
                RichText::new(i18n.t("app.worker_dead_warning")).size(11.5).strong(),
            );
        });
    }

    if !config_warnings.is_empty() {
        ui.add_space(4.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.horizontal(|ui| {
                ui.colored_label(
                    STATUS_WARN,
                    RichText::new(config_warnings.join(" | ")).size(11.0),
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
        ui.add_space(4.0);
        compact_card(t).show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.set_max_width(card_inner_w);
            ui.colored_label(
                STATUS_WARN,
                RichText::new(i18n.t_fmt("board.experimental_title", &[("board", &st.board_id)]))
                    .size(11.5)
                    .strong(),
            );
            ui.weak(RichText::new(i18n.t("board.experimental_desc")).size(10.5));
            if let Some(diag) = &st.diagnostics {
                let ok_cnt = diag.probes.iter().filter(|p| p.success).count();
                ui.weak(
                    RichText::new(i18n.t_fmt(
                        "board.probe_summary",
                        &[
                            ("ok", &ok_cnt.to_string()),
                            ("total", &diag.probes.len().to_string()),
                        ],
                    ))
                    .size(10.5),
                );
            }
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
                STATUS_WARN,
                RichText::new(i18n.t("app.restart_as_admin")).size(11.5),
            );
            ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
            if ui.small_button(i18n.t("common.apply")).clicked() {
                actions.restart_admin = true;
            }
        });
    }

    ui.add_space(5.0);

    // 3. Width-locked middle cockpit controls (fits cleanly without horizontal or vertical overflow)
    let avail_h = ui.available_height();
    let reserve_footer = 76.0_f32;
    egui::ScrollArea::vertical()
        .id_salt("cockpit_middle_scroll")
        .max_width(content_w)
        .max_height((avail_h - reserve_footer).max(200.0))
        .show(ui, |ui| {
            ui.set_width(content_w);
            ui.set_max_width(content_w);

            // Card A: Performance Mode
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                section_title(ui, i18n.t("thermal.mode"), t);
                ui.add_space(3.0);
                let items = [
                    (PerformanceMode::Eco, i18n.t("thermal.mode_eco")),
                    (PerformanceMode::Quiet, i18n.t("thermal.mode_quiet")),
                    (PerformanceMode::Balanced, i18n.t("thermal.mode_balanced")),
                    (
                        PerformanceMode::Performance,
                        i18n.t("thermal.mode_performance"),
                    ),
                ];
                let w = equal_tile_width(card_inner_w, items.len(), 5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    for (mode, label) in items {
                        let selected = cfg.thermal_fan.performance_mode == mode;
                        if tile_button(ui, &label, selected, w, 27.0, t).clicked() {
                            cfg.thermal_fan.performance_mode = mode;
                            on_save();
                            on_cmd(WorkerCommand::SetPerformanceMode(mode));
                        }
                    }
                });
            });

            ui.add_space(5.0);

            // Card B: Fan Mode (short crisp cockpit labels)
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                section_title(ui, i18n.t("fan.mode"), t);
                ui.add_space(3.0);
                let items = [
                    (FanMode::Auto, i18n.t("cockpit.fan_auto")),
                    (FanMode::Max, i18n.t("cockpit.fan_max")),
                    (FanMode::Manual, i18n.t("cockpit.fan_manual")),
                    (FanMode::CustomCurve, i18n.t("cockpit.fan_curve")),
                ];
                let w = equal_tile_width(card_inner_w, items.len(), 5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    for (mode, label) in items {
                        let selected = cfg.thermal_fan.fan_mode == mode;
                        if tile_button(ui, &label, selected, w, 27.0, t).clicked() {
                            cfg.thermal_fan.fan_mode = mode;
                            on_save();
                            on_cmd(WorkerCommand::SetFanMode {
                                mode,
                                manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                            });
                        }
                    }
                });
                if cfg.thermal_fan.fan_mode == FanMode::Manual {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().slider_width = (card_inner_w - 72.0).max(220.0);
                        let resp = ui.add(
                            Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100)
                                .suffix("%"),
                        );
                        if control_committed(ui, "cockpit_manual_fan", &resp) {
                            on_save();
                            on_cmd(WorkerCommand::SetFanMode {
                                mode: FanMode::Manual,
                                manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                            });
                        }
                    });
                }
            });

            ui.add_space(5.0);

            // Card C: Display Refresh Rate & CPU Turbo Boost
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                ui.horizontal(|ui| {
                    section_title(ui, i18n.t("home.quick_display"), t);
                    if let Some(disp) = &st.display_mode {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(
                                RichText::new(format!("{}x{}", disp.width, disp.height))
                                    .size(11.0),
                            );
                        });
                    }
                });
                ui.add_space(3.0);
                if let Some(disp) = &st.display_mode {
                    if !disp.supported_hz.is_empty() {
                        let compact_hz = disp.supported_hz.len() >= 5;
                        let w = equal_tile_width(card_inner_w, disp.supported_hz.len(), 4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
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

                ui.add_space(5.0);
                section_title(ui, i18n.t("home.quick_cpu_boost"), t);
                ui.add_space(3.0);
                let cur_boost = st
                    .cpu_boost
                    .or(cfg.tuning.cpu_boost_mode)
                    .unwrap_or(CpuBoostMode::Aggressive);
                let boost_items = [
                    (CpuBoostMode::Disabled, i18n.t("cockpit.boost_off")),
                    (CpuBoostMode::Enabled, i18n.t("cockpit.boost_on")),
                    (
                        CpuBoostMode::Aggressive,
                        i18n.t("cockpit.boost_aggressive"),
                    ),
                ];
                let bw = equal_tile_width(card_inner_w, boost_items.len(), 5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    for (mode, label) in boost_items {
                        if tile_button(ui, &label, cur_boost == mode, bw, 25.0, t).clicked() {
                            cfg.tuning.cpu_boost_mode = Some(mode);
                            on_save();
                            on_cmd(WorkerCommand::SetCpuBoost(mode));
                        }
                    }
                });
            });

            ui.add_space(5.0);

            // Card D: Keyboard Lighting
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                ui.horizontal(|ui| {
                    section_title(ui, i18n.t("home.quick_keyboard"), t);
                    if ui
                        .checkbox(&mut cfg.lighting.enabled, i18n.t("common.enabled"))
                        .changed()
                    {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let c0 = &mut cfg.lighting.zones[0];
                        let mut srgb = [c0.r, c0.g, c0.b];
                        let resp = ui.color_edit_button_srgb(&mut srgb);
                        if resp.changed() {
                            let picked = RgbColor {
                                r: srgb[0],
                                g: srgb[1],
                                b: srgb[2],
                            };
                            cfg.lighting.zones = [picked; 4];
                            cfg.lighting.enabled = true;
                        }
                        if control_committed(ui, "cockpit_kbd_color", &resp) {
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyLighting);
                        }
                        let border_color = rgb(t.card_border);
                        for rgb_preset in [
                            RgbColor { r: 34, g: 197, b: 94 },
                            RgbColor { r: 220, g: 38, b: 38 },
                            RgbColor {
                                r: 245,
                                g: 245,
                                b: 255,
                            },
                            RgbColor { r: 0, g: 180, b: 216 },
                        ] {
                            let swatch = Color32::from_rgb(rgb_preset.r, rgb_preset.g, rgb_preset.b);
                            let btn = egui::Button::new("")
                                .fill(swatch)
                                .stroke(Stroke::new(1.0_f32, border_color))
                                .corner_radius(CornerRadius::same(4))
                                .min_size(egui::vec2(18.0, 18.0));
                            if ui.add(btn).clicked() {
                                cfg.lighting.zones = [rgb_preset; 4];
                                cfg.lighting.enabled = true;
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                                on_cmd(WorkerCommand::ApplyLighting);
                            }
                        }
                    });
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(i18n.t("lighting.brightness"));
                    ui.spacing_mut().slider_width = (card_inner_w - 135.0).max(185.0);
                    let resp =
                        ui.add(Slider::new(&mut cfg.lighting.brightness_pct, 0..=100).suffix("%"));
                    if control_committed(ui, "cockpit_kbd_brightness", &resp) {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                });
            });

            ui.add_space(5.0);

            // Card E: Battery Charge Limit Presets + Threshold Slider
            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                ui.horizontal(|ui| {
                    section_title(ui, i18n.t("home.quick_battery"), t);
                    if ui
                        .checkbox(&mut cfg.battery.enabled, i18n.t("common.enabled"))
                        .changed()
                    {
                        if !cfg.battery.enabled {
                            cfg.battery.mode = BatteryChargeMode::Auto;
                        } else if cfg.battery.mode == BatteryChargeMode::Auto {
                            cfg.battery.mode = BatteryChargeMode::Inhibit;
                        }
                        cfg.battery.active_profile_id = None;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let travel_active = cfg.battery.one_time_full_charge;
                        let t_label = i18n.t("cockpit.travel_short");
                        if pill_button(ui, &t_label, travel_active, t).clicked() {
                            cfg.battery.one_time_full_charge = !cfg.battery.one_time_full_charge;
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyBatteryNow);
                        }
                    });
                });

                ui.add_space(3.0);
                let builtins = BatteryProfile::builtins();
                let pw = equal_tile_width(card_inner_w, builtins.len(), 5.0);
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
                        let selected = is_profile_active(cfg, &preset);
                        if tile_button(ui, short_label, selected, pw, 25.0, t).clicked() {
                            apply_battery_profile(cfg, &preset);
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::ApplyBatteryNow);
                        }
                    }
                });

                if !cfg.battery.custom_profiles.is_empty() {
                    ui.add_space(3.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
                        for custom in cfg.battery.custom_profiles.clone() {
                            let selected =
                                cfg.battery.active_profile_id.as_deref() == Some(&custom.id);
                            let label = format!("{} ({}%)", custom.name, custom.stop_threshold_pct);
                            if pill_button(ui, &label, selected, t).clicked() {
                                apply_battery_profile(cfg, &custom);
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                                on_cmd(WorkerCommand::ApplyBatteryNow);
                            }
                        }
                    });
                }

                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = (card_inner_w - 72.0).max(235.0);
                    let resp = ui.add(
                        Slider::new(&mut cfg.battery.stop_threshold_pct, 50..=100).suffix("%"),
                    );
                    if resp.changed() {
                        cfg.battery.start_threshold_pct =
                            cfg.battery.stop_threshold_pct.saturating_sub(5).max(40);
                        cfg.battery.active_profile_id = None;
                    }
                    if control_committed(ui, "cockpit_battery_stop", &resp) {
                        if cfg.battery.stop_threshold_pct >= 100 {
                            cfg.battery.enabled = false;
                            cfg.battery.mode = BatteryChargeMode::Auto;
                        } else if !cfg.battery.enabled {
                            cfg.battery.enabled = true;
                            cfg.battery.mode = BatteryChargeMode::Inhibit;
                        }
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                });
                let off_key = if cfg.battery.enabled && cfg.battery.hold_when_shutdown {
                    "battery.when_off_hold_short"
                } else {
                    "battery.when_off_auto_short"
                };
                ui.weak(RichText::new(i18n.t(off_key)).size(10.5));
            });
        });

    // 4. Bottom Drawer Launcher Bar (Fixed Footer)
    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.set_width(content_w);
        ui.set_max_width(content_w);

        ui.horizontal(|ui| {
            ui.weak(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).size(11.0));
            if let Some((msg, is_err, ts)) = &st.last_toast {
                if ts.elapsed() < Duration::from_secs(6) {
                    let color = if *is_err { STATUS_ERR } else { STATUS_OK };
                    let short = if msg.chars().count() > 28 {
                        format!("{}...", msg.chars().take(25).collect::<String>())
                    } else {
                        msg.clone()
                    };
                    ui.colored_label(color, RichText::new(short).size(11.0));
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().button_padding = egui::vec2(9.0, 3.0);
                if ui
                    .small_button(RichText::new(i18n.t("cockpit.quit")).size(11.5))
                    .clicked()
                {
                    actions.quit_app = true;
                }
                if ui
                    .small_button(RichText::new(i18n.t("cockpit.hide_to_tray")).size(11.5))
                    .clicked()
                {
                    actions.hide_to_tray = true;
                }
            });
        });

        ui.add_space(10.0);

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
                if tile_button(ui, &label, selected, dw, 29.0, t).clicked() {
                    actions.toggle_drawer = Some(d);
                }
            }
        });
        ui.add_space(4.0);
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
