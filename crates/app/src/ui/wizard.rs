use crate::theme::{
    accent_color, compact_card, feature_toggle_button, pill_button, section_title, status_badge,
    status_ok, status_warn,
};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Ui};
use omvix_core::{AppConfig, BatteryChargeMode, BatteryServiceState, I18n};

#[derive(Debug, Default, Clone)]
pub struct WizardState {
    pub step: usize,
}

impl WizardState {
    pub const STEP_COUNT: usize = 5;

    pub fn next(&mut self) {
        if self.step + 1 < Self::STEP_COUNT {
            self.step += 1;
        }
    }

    pub fn prev(&mut self) {
        if self.step > 0 {
            self.step -= 1;
        }
    }

    pub fn progress(&self) -> f32 {
        (self.step.min(Self::STEP_COUNT - 1) + 1) as f32 / Self::STEP_COUNT as f32
    }
}

#[derive(Default)]
pub struct WizardActions {
    pub finished: bool,
    pub skipped: bool,
    pub restart_admin: bool,
    pub language_changed: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &mut I18n,
    wizard: &mut WizardState,
    pane_width: f32,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) -> WizardActions {
    let mut actions = WizardActions::default();
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    let content_w = (pane_width - 24.0).max(240.0);
    let card_inner_w = (content_w - 22.0).max(200.0);
    ui.set_width(content_w);
    ui.set_max_width(content_w);
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);

    // Top bar: Title, step counter & quick language toggle
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(i18n.t("wizard.title"))
                    .size(17.5)
                    .strong()
                    .color(accent),
            );
            let step_str = (wizard.step.min(WizardState::STEP_COUNT - 1) + 1).to_string();
            let total_str = WizardState::STEP_COUNT.to_string();
            ui.weak(
                RichText::new(i18n.t_fmt(
                    "wizard.step_indicator",
                    &[("step", &step_str), ("total", &total_str)],
                ))
                .size(11.5),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (code, label) in [("en", "EN"), ("tr", "TR")] {
                let selected = cfg.general.language.eq_ignore_ascii_case(code);
                if pill_button(ui, label, selected, t).clicked() && !selected {
                    cfg.general.language = code.to_string();
                    i18n.set_language(code);
                    on_save();
                    actions.language_changed = true;
                }
            }
        });
    });

    ui.add(
        egui::ProgressBar::new(wizard.progress())
            .desired_width(content_w)
            .desired_height(6.0),
    );
    ui.add_space(4.0);

    // Middle scrollable wizard content
    let avail_h = ui.available_height();
    let reserve_footer = 56.0_f32;
    egui::ScrollArea::vertical()
        .id_salt("omvix_wizard_scroll")
        .max_width(content_w)
        .max_height((avail_h - reserve_footer).max(220.0))
        .show(ui, |ui| {
            ui.set_width(content_w);
            ui.set_max_width(content_w);

            compact_card(t).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);

                match wizard.step {
                    0 => render_step_board(ui, cfg, st, i18n, t, &mut actions, on_cmd, on_save),
                    1 => render_step_autostart(ui, cfg, st, i18n, on_cmd, on_save),
                    2 => render_step_pawnio(ui, st, i18n, t, on_cmd),
                    3 => render_step_battery(ui, cfg, st, i18n, t, on_cmd, on_save),
                    _ => render_step_summary(ui, cfg, st, i18n),
                }
            });
        });

    // Bottom navigation footer
    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.set_width(content_w);
        ui.set_max_width(content_w);

        ui.horizontal(|ui| {
            if ui.button(i18n.t("wizard.skip")).clicked() {
                actions.skipped = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if wizard.step + 1 < WizardState::STEP_COUNT {
                    if pill_button(ui, &i18n.t("wizard.next"), true, t).clicked() {
                        wizard.next();
                    }
                } else if pill_button(ui, &i18n.t("wizard.finish"), true, t).clicked() {
                    actions.finished = true;
                }

                if ui
                    .add_enabled(wizard.step > 0, egui::Button::new(i18n.t("wizard.back")))
                    .clicked()
                {
                    wizard.prev();
                }
            });
        });
    });

    actions
}

#[allow(clippy::too_many_arguments)]
fn render_step_board(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    t: &omvix_core::CustomThemeConfig,
    actions: &mut WizardActions,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    section_title(ui, i18n.t("wizard.step1_title"), t);
    ui.label(i18n.t("wizard.step1_desc"));
    ui.add_space(6.0);

    let model = st
        .diagnostics
        .as_ref()
        .map(|d| d.identity.model.trim())
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| i18n.t("wizard.detecting"));
    ui.label(RichText::new(model).strong().size(13.5));

    ui.horizontal_wrapped(|ui| {
        let board_disp = if st.board_id.is_empty() {
            i18n.t("wizard.detecting")
        } else {
            st.board_id.clone()
        };
        ui.label(format!("{}: {board_disp}", i18n.t("diagnostics.board_id")));
        if !st.board_id.is_empty() {
            if st.board_known {
                status_badge(ui, &i18n.t("wizard.board_verified"), status_ok(t));
            } else {
                status_badge(ui, &i18n.t("wizard.board_unknown"), status_warn(t));
            }
        }
    });

    ui.horizontal_wrapped(|ui| {
        if st.security.is_admin {
            status_badge(ui, &i18n.t("app.admin_active"), status_ok(t));
        } else {
            status_badge(ui, &i18n.t("wizard.standard_user"), status_warn(t));
            if ui.button(i18n.t("app.restart_as_admin")).clicked() {
                actions.restart_admin = true;
            }
        }
    });

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
            .size(11.5),
        );
    }

    if !st.board_known && !st.bios_writes_allowed && !st.board_id.is_empty() {
        ui.add_space(6.0);
        ui.weak(RichText::new(i18n.t("board.experimental_desc")).size(11.0));
        if pill_button(ui, &i18n.t("board.approve_experimental"), false, t).clicked() {
            cfg.general.experimental_board_id = Some(st.board_id.clone());
            on_save();
            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
        }
    }
}

fn render_step_autostart(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    section_title(ui, i18n.t("wizard.step2_title"), t);
    ui.label(i18n.t("wizard.step2_desc"));
    ui.add_space(6.0);

    if feature_toggle_button(
        ui,
        &i18n.t("wizard.autostart_toggle"),
        &mut cfg.general.start_with_windows,
        &i18n.t("common.enabled"),
        t,
    )
    .changed()
    {
        on_save();
        on_cmd(WorkerCommand::SetAutostart(cfg.general.start_with_windows));
    }

    ui.add_space(4.0);
    if feature_toggle_button(
        ui,
        &i18n.t("settings.start_minimized"),
        &mut cfg.general.start_minimized,
        &i18n.t("common.enabled"),
        t,
    )
    .changed()
    {
        on_save();
    }

    ui.add_space(4.0);
    ui.weak(RichText::new(i18n.t("settings.autostart_admin_note")).size(11.0));
    if !st.installed_copy_status.installed_path.is_empty() {
        ui.add_space(4.0);
        ui.weak(format!("{}:", i18n.t("settings.installed_copy_path")));
        ui.monospace(&st.installed_copy_status.installed_path);
    }
}

fn render_step_pawnio(
    ui: &mut Ui,
    st: &SharedState,
    i18n: &I18n,
    t: &omvix_core::CustomThemeConfig,
    on_cmd: &mut dyn FnMut(WorkerCommand),
) {
    section_title(ui, i18n.t("wizard.step3_title"), t);
    ui.label(i18n.t("wizard.step3_desc"));
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        if st.pawnio_cpu_active {
            status_badge(ui, &i18n.t("monitor.pawnio_active"), status_ok(t));
            if let Some(temp) = st.telemetry.cpu_temp_c {
                ui.label(RichText::new(format!("{temp:.1} °C")).strong());
            }
        } else {
            status_badge(ui, &i18n.t("common.disabled"), status_warn(t));
        }
    });

    ui.add_space(6.0);
    if st.pawnio_installing {
        ui.weak(i18n.t("monitor.installing_pawnio"));
    } else if !st.pawnio_cpu_active
        && pill_button(ui, &i18n.t("monitor.install_pawnio"), true, t).clicked()
    {
        on_cmd(WorkerCommand::InstallPawnIo);
    }

    ui.add_space(6.0);
    ui.weak(RichText::new(i18n.t("wizard.step3_optional_note")).size(11.0));
}

fn render_step_battery(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    t: &omvix_core::CustomThemeConfig,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    section_title(ui, i18n.t("wizard.step4_title"), t);
    ui.label(i18n.t("wizard.step4_desc"));
    ui.add_space(6.0);

    ui.colored_label(
        status_warn(t),
        RichText::new(i18n.t("wizard.step4_hold_warning"))
            .size(11.5)
            .strong(),
    );
    ui.add_space(6.0);

    let presets = [
        ("desktop", "battery.preset_desktop", 60u8, 55u8, true),
        ("balanced", "battery.preset_balanced", 80, 75, true),
        ("travel", "battery.preset_travel", 90, 85, true),
        ("full", "battery.preset_full", 100, 95, false),
    ];
    ui.horizontal_wrapped(|ui| {
        for (id, key, stop, start, en) in presets {
            let selected =
                cfg.battery.enabled == en && (!en || cfg.battery.stop_threshold_pct == stop);
            if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                cfg.battery.enabled = en;
                cfg.battery.stop_threshold_pct = stop;
                cfg.battery.start_threshold_pct = start;
                cfg.battery.mode = if en {
                    BatteryChargeMode::Inhibit
                } else {
                    BatteryChargeMode::Auto
                };
                cfg.battery.active_profile_id = Some(id.to_string());
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        }
    });

    ui.add_space(6.0);
    if feature_toggle_button(
        ui,
        &i18n.t("battery.charge_limit"),
        &mut cfg.battery.enabled,
        &i18n.t("common.enabled"),
        t,
    )
    .changed()
    {
        if cfg.battery.enabled {
            if cfg.battery.mode == BatteryChargeMode::Auto {
                cfg.battery.mode = BatteryChargeMode::Inhibit;
            }
        } else {
            cfg.battery.mode = BatteryChargeMode::Auto;
            cfg.battery.active_profile_id = None;
        }
        on_save();
        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
    }

    ui.add_space(4.0);
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

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{}:", i18n.t("battery.service_name_label")));
        match st.battery_service.state {
            BatteryServiceState::Running => {
                status_badge(ui, &i18n.t("battery.service_state_running"), status_ok(t));
            }
            BatteryServiceState::Stopped => {
                status_badge(ui, &i18n.t("battery.service_state_stopped"), status_warn(t));
            }
            BatteryServiceState::Starting | BatteryServiceState::Stopping => {
                status_badge(
                    ui,
                    &i18n.t("battery.service_state_transitioning"),
                    status_warn(t),
                );
            }
            BatteryServiceState::NotInstalled | BatteryServiceState::Unknown => {
                status_badge(
                    ui,
                    &i18n.t("battery.service_state_not_installed"),
                    status_warn(t),
                );
                if st.security.is_admin
                    && !st.battery_service_busy
                    && pill_button(ui, &i18n.t("battery.service_install"), false, t).clicked()
                {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::InstallBatteryService);
                }
            }
        }
    });
}

fn render_step_summary(ui: &mut Ui, cfg: &AppConfig, st: &SharedState, i18n: &I18n) {
    section_title(ui, i18n.t("wizard.step5_title"), &cfg.general.custom_theme);
    ui.label(i18n.t("wizard.step5_desc"));
    ui.add_space(6.0);

    let board_val = if st.board_id.is_empty() {
        i18n.t("wizard.detecting")
    } else if st.board_known {
        format!("{} ({})", st.board_id, i18n.t("wizard.board_verified"))
    } else if st.bios_writes_allowed {
        format!("{} (Experimental)", st.board_id)
    } else {
        format!("{} ({})", st.board_id, i18n.t("wizard.board_unknown"))
    };
    ui.label(format!("• {}: {board_val}", i18n.t("wizard.summary_board")));

    let auto_val = if cfg.general.start_with_windows {
        i18n.t("common.enabled")
    } else {
        i18n.t("common.disabled")
    };
    ui.label(format!(
        "• {}: {auto_val}",
        i18n.t("wizard.summary_autostart")
    ));

    let pawn_val = if st.pawnio_cpu_active {
        i18n.t("monitor.pawnio_active")
    } else {
        i18n.t("common.disabled")
    };
    ui.label(format!("• {}: {pawn_val}", i18n.t("wizard.summary_pawnio")));

    let bat_val = if cfg.battery.enabled {
        let off_key = if cfg.battery.hold_when_shutdown {
            "battery.when_off_hold_short"
        } else {
            "battery.when_off_auto_short"
        };
        format!("{}% ({})", cfg.battery.stop_threshold_pct, i18n.t(off_key))
    } else {
        i18n.t("common.disabled")
    };
    ui.label(format!("• {}: {bat_val}", i18n.t("wizard.summary_battery")));

    ui.add_space(8.0);
    ui.weak(RichText::new(i18n.t("wizard.safe_mode_tip")).size(11.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_state_navigation_and_progress_stay_bounded() {
        let mut ws = WizardState::default();
        assert_eq!(ws.step, 0);
        assert!((ws.progress() - 0.2).abs() < 1e-6);

        ws.prev();
        assert_eq!(ws.step, 0);

        for expected in 1..WizardState::STEP_COUNT {
            ws.next();
            assert_eq!(ws.step, expected);
        }
        assert!((ws.progress() - 1.0).abs() < 1e-6);

        // Next past last step clamps cleanly
        ws.next();
        assert_eq!(ws.step, WizardState::STEP_COUNT - 1);
        assert!((ws.progress() - 1.0).abs() < 1e-6);

        ws.prev();
        assert_eq!(ws.step, WizardState::STEP_COUNT - 2);
        assert!((ws.progress() - 0.8).abs() < 1e-6);
    }
}
