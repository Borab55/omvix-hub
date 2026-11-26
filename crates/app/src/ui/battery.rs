use crate::theme::{
    accent_color, card_frame, feature_toggle_button, muted_text, pill_button, section_title,
    section_toggle_button, status_badge, status_err, status_ok, status_warn,
};
use crate::ui::charts::battery_health_chart;
use crate::ui::cockpit::{apply_battery_profile, is_profile_active};
use crate::ui::{control_committed, open_folder_in_explorer, unique_profile_slug};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, BatteryChargeMode, BatteryProfile, BatteryServiceState, I18n};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BatteryDraftState {
    pub profile_name: String,
    pub stop_pct: u8,
    pub start_pct: u8,
    pub mode: BatteryChargeMode,
    pub confirm_uninstall_service: bool,
}

impl Default for BatteryDraftState {
    fn default() -> Self {
        Self {
            profile_name: String::new(),
            stop_pct: 75,
            start_pct: 70,
            mode: BatteryChargeMode::Inhibit,
            confirm_uninstall_service: false,
        }
    }
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    draft: &mut BatteryDraftState,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);
    let ok_c = status_ok(t);
    let warn_c = status_warn(t);
    let err_c = status_err(t);
    let muted_c = muted_text(t);

    ui.heading(i18n.t("nav.battery"));
    ui.add_space(6.0);

    if st.hp_charge_service_running {
        card_frame(t).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, "hp-charge-limit-rs", warn_c);
                ui.label(i18n.t("battery.conflict_service_warning"));
            });
        });
        ui.add_space(8.0);
    }

    // Live Battery Status Card
    card_frame(t).show(ui, |ui| {
        if let Some(b) = &st.battery_status {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{}%", b.charge_percent))
                        .size(26.0)
                        .strong()
                        .color(accent),
                );
                ui.add_space(10.0);
                let src = if b.ac_connected {
                    i18n.t("dashboard.power_source_ac")
                } else {
                    i18n.t("dashboard.power_source_dc")
                };
                let mode_text = match b.current_mode {
                    BatteryChargeMode::Auto => i18n.t("battery.mode_auto_short"),
                    BatteryChargeMode::Inhibit => i18n.t("battery.mode_inhibit_short"),
                    BatteryChargeMode::Hold => i18n.t("battery.mode_hold_short"),
                    BatteryChargeMode::Unknown => i18n.t("battery.mode_unknown"),
                };
                status_badge(
                    ui,
                    &format!("{src} | {mode_text}"),
                    if b.charging || b.current_mode == BatteryChargeMode::Unknown {
                        warn_c
                    } else {
                        ok_c
                    },
                );
                let off_badge_key = if cfg.battery.enabled && cfg.battery.hold_when_shutdown {
                    "battery.when_off_hold_short"
                } else {
                    "battery.when_off_auto_short"
                };
                status_badge(
                    ui,
                    &i18n.t(off_badge_key),
                    if cfg.battery.enabled && cfg.battery.hold_when_shutdown {
                        warn_c
                    } else {
                        ok_c
                    },
                );
            });

            ui.add(egui::ProgressBar::new(b.charge_percent as f32 / 100.0).show_percentage());
            ui.add_space(6.0);

            egui::Grid::new("battery_metrics_grid")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    ui.label(i18n.t("battery.wattage"));
                    ui.label(
                        b.charge_rate_w
                            .map(|w| format!("{w:+.2} W"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("battery.voltage"));
                    ui.label(
                        b.voltage_v
                            .map(|v| format!("{v:.2} V"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("battery.remaining_cap"));
                    ui.label(
                        b.remaining_capacity_mwh
                            .map(|m| format!("{m} mWh"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("battery.time_remaining"));
                    ui.label(
                        b.time_remaining_min
                            .map(|m| format!("{}h {}m", m / 60, m % 60))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("battery.health"));
                    let cap_str = match (b.full_charge_capacity_mwh, b.design_capacity_mwh) {
                        (Some(f), Some(d)) if d > 0 => {
                            let health_pct = ((f as f32 / d as f32) * 100.0).clamp(0.0, 100.0);
                            format!("{f} / {d} mWh ({health_pct:.1}%)")
                        }
                        (Some(f), Some(d)) => format!("{f} / {d} mWh"),
                        _ => "N/A".into(),
                    };
                    ui.label(cap_str);
                    ui.end_row();

                    ui.label(i18n.t("battery.wear_level"));
                    ui.label(
                        b.wear_percent
                            .map(|w| format!("{w:.1}%"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();
                });
        } else {
            ui.weak("...");
        }
    });

    ui.add_space(8.0);

    // Weekly Battery Health / Wear History & Chart Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("battery.health_history_title"), t);
            status_badge(ui, &st.battery_health_history.len().to_string(), accent);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(i18n.t("battery.log_snapshot_now")).clicked() {
                    on_cmd(WorkerCommand::RecordBatteryHealthNow);
                }
            });
        });
        ui.add_space(4.0);
        if st.battery_health_history.is_empty() {
            ui.weak(i18n.t("battery.health_history_empty"));
        } else {
            battery_health_chart(
                ui,
                &st.battery_health_history,
                t,
                &i18n.t("battery.health"),
                &i18n.t("battery.wear_level"),
            );
            ui.add_space(4.0);
            ui.collapsing(i18n.t("battery.health_history_table"), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("battery_health_history_hscroll")
                    .show(ui, |ui| {
                        egui::Grid::new("battery_health_history_grid")
                            .striped(true)
                            .num_columns(4)
                            .spacing([14.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong(i18n.t("battery.history_date"));
                                ui.strong(i18n.t("battery.history_capacity"));
                                ui.strong(i18n.t("battery.health"));
                                ui.strong(i18n.t("battery.wear_level"));
                                ui.end_row();
                                for rec in st.battery_health_history.iter().rev().take(16) {
                                    ui.monospace(&rec.date_label);
                                    ui.monospace(format!(
                                        "{} / {} mWh",
                                        rec.full_charge_capacity_mwh, rec.design_capacity_mwh
                                    ));
                                    ui.label(format!("{:.1}%", rec.health_pct));
                                    ui.label(format!("{:.1}%", rec.wear_pct));
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
    });

    ui.add_space(8.0);

    // Built-in Battery Charge Presets (inspired by Borab55/hp-charge-limiter)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("battery.presets_title"), t);
        ui.add_space(6.0);
        let builtins = BatteryProfile::builtins();
        ui.columns(2, |cols| {
            for (idx, preset) in builtins.into_iter().enumerate() {
                let col = &mut cols[idx % 2];
                let (title_key, desc_key) = match preset.id.as_str() {
                    "desktop" => ("battery.preset_desktop", "battery.preset_desktop_desc"),
                    "balanced" => ("battery.preset_balanced", "battery.preset_balanced_desc"),
                    "travel" => ("battery.preset_travel", "battery.preset_travel_desc"),
                    "full" => ("battery.preset_full", "battery.preset_full_desc"),
                    _ => ("battery.preset_balanced", "battery.preset_balanced_desc"),
                };
                let selected = is_profile_active(cfg, &preset);
                col.vertical(|ui| {
                    if pill_button(ui, &i18n.t(title_key), selected, t).clicked() {
                        apply_battery_profile(cfg, &preset);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                    ui.weak(i18n.t(desc_key));
                    ui.add_space(4.0);
                });
            }
        });
    });

    ui.add_space(8.0);

    // Custom User Profiles Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("battery.custom_profiles"), t);
        ui.add_space(4.0);

        if cfg.battery.custom_profiles.is_empty() {
            ui.weak(i18n.t("battery.no_custom_profiles"));
        } else {
            let mut apply_profile: Option<BatteryProfile> = None;
            let mut delete_idx: Option<usize> = None;

            for (idx, prof) in cfg.battery.custom_profiles.iter().enumerate() {
                let is_active = cfg.battery.active_profile_id.as_deref() == Some(&prof.id);
                ui.push_id(&prof.id, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if pill_button(ui, &prof.name, is_active, t).clicked() {
                            apply_profile = Some(prof.clone());
                        }
                        ui.weak(format!(
                            "{}% / {}%  •  {:?}",
                            prof.stop_threshold_pct, prof.start_threshold_pct, prof.mode
                        ));
                        if ui.button(i18n.t("battery.delete")).clicked() {
                            delete_idx = Some(idx);
                        }
                    });
                });
            }

            if let Some(p) = apply_profile {
                apply_battery_profile(cfg, &p);
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

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.profile_name"));
            ui.add(
                egui::TextEdit::singleline(&mut draft.profile_name)
                    .desired_width(200.0)
                    .clip_text(true),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.stop_threshold"));
            ui.add(Slider::new(&mut draft.stop_pct, 50..=100).suffix("%"));
        });
        let draft_max_start = draft.stop_pct.saturating_sub(2).max(40);
        if draft.start_pct > draft_max_start {
            draft.start_pct = draft_max_start;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.start_threshold"));
            ui.add(Slider::new(&mut draft.start_pct, 40..=draft_max_start).suffix("%"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.hold_mode"));
            for (m, key) in [
                (BatteryChargeMode::Inhibit, "battery.mode_inhibit"),
                (BatteryChargeMode::Hold, "battery.mode_hold"),
                (BatteryChargeMode::Auto, "battery.mode_auto"),
            ] {
                if pill_button(ui, &i18n.t(key), draft.mode == m, t).clicked() {
                    draft.mode = m;
                }
            }
        });
        if ui
            .button(RichText::new(i18n.t("battery.save_profile")).strong())
            .clicked()
        {
            let clean_name = draft.profile_name.trim();
            let final_name = if clean_name.is_empty() {
                format!("Custom {}%", draft.stop_pct)
            } else {
                clean_name.to_string()
            };
            let candidate = unique_profile_slug(&final_name, "custom", |id| {
                cfg.battery.custom_profiles.iter().any(|p| p.id == id)
                    || BatteryProfile::builtins().iter().any(|p| p.id == id)
            });
            let prof = BatteryProfile {
                id: candidate,
                name: final_name,
                enabled: draft.mode != BatteryChargeMode::Auto,
                stop_threshold_pct: draft.stop_pct,
                start_threshold_pct: draft.start_pct,
                mode: draft.mode,
                builtin: false,
            };
            apply_battery_profile(cfg, &prof);
            cfg.battery.custom_profiles.push(prof);
            draft.profile_name.clear();
            on_save();
            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            on_cmd(WorkerCommand::ApplyBatteryNow);
        }
    });

    ui.add_space(8.0);

    // Manual Charge Limiting & Travel Mode Card
    card_frame(t).show(ui, |ui| {
        if section_toggle_button(
            ui,
            &i18n.t("battery.charge_limit"),
            &mut cfg.battery.enabled,
            &i18n.t("common.enabled"),
            t,
        )
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

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.hold_mode"));
            for (m, key) in [
                (BatteryChargeMode::Inhibit, "battery.mode_inhibit"),
                (BatteryChargeMode::Hold, "battery.mode_hold"),
                (BatteryChargeMode::Auto, "battery.mode_auto"),
            ] {
                if pill_button(ui, &i18n.t(key), cfg.battery.mode == m, t).clicked() {
                    cfg.battery.mode = m;
                    cfg.battery.enabled = m != BatteryChargeMode::Auto;
                    cfg.battery.active_profile_id = None;
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyBatteryNow);
                }
            }
        });

        let mut commit_battery = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.stop_threshold"));
            let stop_resp =
                ui.add(Slider::new(&mut cfg.battery.stop_threshold_pct, 50..=100).suffix("%"));
            if stop_resp.changed() {
                cfg.battery.active_profile_id = None;
            }
            if control_committed(ui, "battery_drawer_stop", &stop_resp) {
                commit_battery = true;
            }
        });

        let max_start = cfg.battery.stop_threshold_pct.saturating_sub(2).max(40);
        if cfg.battery.start_threshold_pct > max_start {
            cfg.battery.start_threshold_pct = max_start;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("battery.start_threshold"));
            let start_resp = ui
                .add(Slider::new(&mut cfg.battery.start_threshold_pct, 40..=max_start).suffix("%"));
            if start_resp.changed() {
                cfg.battery.active_profile_id = None;
            }
            if control_committed(ui, "battery_drawer_start", &start_resp) {
                commit_battery = true;
            }
        });
        if commit_battery {
            on_save();
            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            on_cmd(WorkerCommand::ApplyBatteryNow);
        }

        ui.add_space(6.0);
        if feature_toggle_button(
            ui,
            &i18n.t("battery.hold_when_shutdown"),
            &mut cfg.battery.hold_when_shutdown,
            &i18n.t("common.enabled"),
            t,
        )
        .changed()
        {
            on_save();
            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
        }
        ui.weak(RichText::new(i18n.t("battery.hold_when_shutdown_desc")).size(11.0));

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let now_utc = chrono::Utc::now().timestamp();
            let travel_active = cfg.battery.is_travel_charge_active(now_utc);
            let travel_btn_label = if travel_active {
                format!("✓ {}", i18n.t("battery.one_time_full_charge"))
            } else {
                i18n.t("battery.one_time_full_charge")
            };
            if pill_button(ui, &travel_btn_label, travel_active, t).clicked() {
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

        if cfg
            .battery
            .is_travel_charge_active(chrono::Utc::now().timestamp())
        {
            ui.add_space(4.0);
            ui.colored_label(warn_c, i18n.t("battery.one_time_active"));
        } else {
            ui.weak(i18n.t("battery.travel_mode_desc"));
        }

        if st.battery_service.state == BatteryServiceState::Running {
            ui.add_space(4.0);
            ui.colored_label(
                ok_c,
                RichText::new(i18n.t("battery.service_active_sync_note")).size(11.5),
            );
        }
    });

    ui.add_space(8.0);

    // Optional Standalone Windows Background Service (Zero-Trace Uninstallable)
    card_frame(t).show(ui, |ui| {
        let svc = &st.battery_service;
        let (state_text, state_color) = match svc.state {
            BatteryServiceState::Running => (i18n.t("battery.service_state_running"), ok_c),
            BatteryServiceState::Stopped => (i18n.t("battery.service_state_stopped"), warn_c),
            BatteryServiceState::Starting | BatteryServiceState::Stopping => {
                (i18n.t("battery.service_state_transitioning"), warn_c)
            }
            BatteryServiceState::NotInstalled | BatteryServiceState::Unknown => {
                (i18n.t("battery.service_state_not_installed"), muted_c)
            }
        };

        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("battery.service_title"), t);
            status_badge(ui, &state_text, state_color);
        });
        ui.add_space(4.0);
        ui.weak(i18n.t("battery.service_desc"));
        ui.add_space(6.0);

        let exe_display = svc
            .active_binary_path
            .as_deref()
            .unwrap_or(&svc.installed_exe_path);

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(i18n.t("battery.service_name_label")).strong());
            ui.monospace(format!("{}  (LocalSystem · AutoStart)", svc.service_name));
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(i18n.t("battery.service_exe_path")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if svc.exe_exists
                    && ui
                        .push_id("open_svc_exe_dir", |ui| {
                            ui.small_button(i18n.t("battery.service_open_folder"))
                        })
                        .inner
                        .clicked()
                {
                    if let Some(dir) = Path::new(&svc.installed_exe_path).parent() {
                        open_folder_in_explorer(dir);
                    }
                }
            });
        });
        ui.add(
            egui::Label::new(
                RichText::new(exe_display)
                    .monospace()
                    .size(11.5)
                    .color(muted_text(t)),
            )
            .wrap(),
        );

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(i18n.t("battery.service_data_path")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if svc.data_dir_exists
                    && ui
                        .push_id("open_svc_data_dir", |ui| {
                            ui.small_button(i18n.t("battery.service_open_folder"))
                        })
                        .inner
                        .clicked()
                {
                    open_folder_in_explorer(Path::new(&svc.data_dir_path));
                }
            });
        });
        ui.add(
            egui::Label::new(
                RichText::new(&svc.data_dir_path)
                    .monospace()
                    .size(11.5)
                    .color(muted_text(t)),
            )
            .wrap(),
        );

        ui.add_space(8.0);
        if cfg.battery.enabled
            && cfg.battery.hold_when_shutdown
            && svc.state != BatteryServiceState::Running
        {
            ui.colored_label(
                accent,
                RichText::new(i18n.t("battery.service_recommended_note")).size(11.5),
            );
            ui.add_space(4.0);
        }
        if st.battery_service_busy {
            ui.colored_label(warn_c, i18n.t("battery.service_busy"));
        }

        ui.add_enabled_ui(!st.battery_service_busy, |ui| {
            let installed = svc.state != BatteryServiceState::NotInstalled;
            if !installed {
                ui.horizontal_wrapped(|ui| {
                    if pill_button(ui, &i18n.t("battery.service_install"), true, t).clicked() {
                        draft.confirm_uninstall_service = false;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::InstallBatteryService);
                    }
                    if (svc.exe_exists || svc.data_dir_exists)
                        && ui
                            .button(
                                RichText::new(i18n.t("battery.service_clean_leftovers"))
                                    .color(err_c),
                            )
                            .clicked()
                    {
                        on_cmd(WorkerCommand::UninstallBatteryServiceZeroTrace);
                    }
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    if pill_button(ui, &i18n.t("battery.service_update_sync"), false, t).clicked() {
                        draft.confirm_uninstall_service = false;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::InstallBatteryService);
                    }
                    if svc.state == BatteryServiceState::Stopped
                        && ui.button(i18n.t("battery.service_start")).clicked()
                    {
                        on_cmd(WorkerCommand::StartBatteryService);
                    }
                    if svc.state == BatteryServiceState::Running {
                        if ui.button(i18n.t("battery.service_restart")).clicked() {
                            on_cmd(WorkerCommand::RestartBatteryService);
                        }
                        if ui.button(i18n.t("battery.service_stop")).clicked() {
                            on_cmd(WorkerCommand::StopBatteryService);
                        }
                    }
                });

                ui.add_space(6.0);
                if !draft.confirm_uninstall_service {
                    if ui
                        .button(
                            RichText::new(i18n.t("battery.service_uninstall_zero")).color(err_c),
                        )
                        .clicked()
                    {
                        draft.confirm_uninstall_service = true;
                    }
                } else {
                    ui.weak(
                        RichText::new(i18n.t("battery.service_uninstall_confirm_desc")).size(11.0),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .button(
                                RichText::new(i18n.t("battery.service_uninstall_confirm_btn"))
                                    .strong()
                                    .color(err_c),
                            )
                            .clicked()
                        {
                            draft.confirm_uninstall_service = false;
                            cfg.battery.enabled = false;
                            cfg.battery.mode = BatteryChargeMode::Auto;
                            cfg.battery.active_profile_id = None;
                            cfg.battery.clear_travel_charge();
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            on_cmd(WorkerCommand::UninstallBatteryServiceZeroTrace);
                        }
                        if ui.button(i18n.t("common.cancel")).clicked() {
                            draft.confirm_uninstall_service = false;
                        }
                    });
                }
            }
        });
    });

    ui.add_space(8.0);

    // R5 Recovery Guide Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("battery.recovery_title"), t);
        ui.add_space(4.0);
        ui.weak(RichText::new(i18n.t("battery.recovery_desc")).size(11.0));
        ui.add_space(3.0);
        ui.monospace("omvix-cli.exe battery auto");
    });
}
