use crate::theme::{card_frame, section_title, status_badge, STATUS_ERR, STATUS_OK, STATUS_WARN};
use crate::worker::WorkerCommand;
use egui::{RichText, Ui};
use omvix_core::config::{is_valid_custom_command_syntax, OmenKeyAction};
use omvix_core::{AppConfig, I18n};

#[cfg(windows)]
use omvix_windows::hotkeys::parse_shortcut;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HotkeyBindStatus {
    #[default]
    Disabled,
    Bound(String),
    Conflict(String),
    Invalid(String),
    Error(String),
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &I18n,
    bind_status: &HotkeyBindStatus,
    config_warnings: &[String],
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
    on_rebind_hotkeys: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();

    ui.heading(i18n.t("nav.hotkeys"));
    ui.add_space(6.0);

    // Omen Key Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, i18n.t("hotkeys.omen_key"), t);
            if ui
                .checkbox(&mut cfg.hotkeys.capture_omen_key, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_rebind_hotkeys();
            }
        });

        ui.add_enabled_ui(cfg.hotkeys.capture_omen_key, |ui| {
            for (action, key) in [
                (OmenKeyAction::ToggleWindow, "hotkeys.action_toggle_window"),
                (
                    OmenKeyAction::CyclePerformanceMode,
                    "hotkeys.action_cycle_mode",
                ),
                (
                    OmenKeyAction::LaunchCustomCommand,
                    "hotkeys.action_custom_app",
                ),
            ] {
                if ui
                    .radio_value(&mut cfg.hotkeys.omen_key_action, action, i18n.t(key))
                    .changed()
                {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            }

            if cfg.hotkeys.omen_key_action == OmenKeyAction::LaunchCustomCommand {
                if !config_warnings.is_empty() {
                    ui.colored_label(
                        STATUS_WARN,
                        RichText::new(config_warnings.join(" | ")).size(11.5),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n.t("hotkeys.custom_command"));
                    if ui
                        .text_edit_singleline(&mut cfg.hotkeys.custom_command)
                        .changed()
                    {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                    let cmd_trimmed = cfg.hotkeys.custom_command.trim();
                    let cmd_valid =
                        !cmd_trimmed.is_empty() && is_valid_custom_command_syntax(cmd_trimmed);
                    if !cmd_trimmed.is_empty() {
                        if cmd_valid {
                            status_badge(ui, &i18n.t("common.status_ok"), STATUS_OK);
                        } else {
                            status_badge(ui, &i18n.t("common.status_error"), STATUS_ERR);
                        }
                    }
                    if ui
                        .add_enabled(cmd_valid, egui::Button::new(i18n.t("hotkeys.test_command")))
                        .clicked()
                    {
                        on_cmd(WorkerCommand::LaunchCustomCommand {
                            command: cfg.hotkeys.custom_command.clone(),
                            args: cfg.hotkeys.custom_args.clone(),
                        });
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n.t("hotkeys.custom_args"));
                    if ui
                        .text_edit_singleline(&mut cfg.hotkeys.custom_args)
                        .changed()
                    {
                        cfg.hotkeys
                            .custom_args
                            .retain(|c| !matches!(c, '\0' | '\r' | '\n'));
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                });
                ui.weak(RichText::new(i18n.t("hotkeys.custom_command_hint")).size(11.0));
            }
        });
    });

    ui.add_space(8.0);

    // Global Shortcut Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("hotkeys.global_shortcut"), t);
        ui.horizontal_wrapped(|ui| {
            let (badge_text, badge_color) = match bind_status {
                HotkeyBindStatus::Disabled => {
                    (i18n.t("common.disabled"), ui.visuals().weak_text_color())
                }
                HotkeyBindStatus::Bound(_) => (i18n.t("hotkeys.status_active"), STATUS_OK),
                HotkeyBindStatus::Conflict(_) => (i18n.t("hotkeys.status_conflict"), STATUS_WARN),
                HotkeyBindStatus::Invalid(_) | HotkeyBindStatus::Error(_) => {
                    (i18n.t("common.status_error"), STATUS_ERR)
                }
            };
            status_badge(ui, &badge_text, badge_color);
            if let HotkeyBindStatus::Bound(combo) = bind_status {
                ui.weak(combo);
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let shortcut_resp = ui.text_edit_singleline(&mut cfg.hotkeys.global_toggle_shortcut);

            #[cfg(windows)]
            let valid = {
                let valid = parse_shortcut(&cfg.hotkeys.global_toggle_shortcut).is_ok();
                if valid {
                    status_badge(ui, &i18n.t("common.status_ok"), STATUS_OK);
                } else {
                    status_badge(ui, &i18n.t("common.status_error"), STATUS_ERR);
                }
                valid
            };
            #[cfg(not(windows))]
            let valid = true;

            let enter_applied = shortcut_resp.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && valid;

            if ui.button(i18n.t("common.apply")).clicked() || enter_applied {
                on_save();
                on_rebind_hotkeys();
            }
        });
        match bind_status {
            HotkeyBindStatus::Conflict(combo) => {
                ui.add_space(4.0);
                ui.colored_label(
                    STATUS_WARN,
                    RichText::new(
                        i18n.t("hotkeys.conflict_detail")
                            .replace("{shortcut}", combo),
                    )
                    .size(12.0),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.weak(i18n.t("hotkeys.conflict_alternatives"));
                    for alt in ["Ctrl+Alt+O", "Ctrl+Shift+F12", "Alt+F12"] {
                        if ui.small_button(alt).clicked() {
                            cfg.hotkeys.global_toggle_shortcut = alt.to_string();
                            on_save();
                            on_rebind_hotkeys();
                        }
                    }
                });
            }
            HotkeyBindStatus::Invalid(msg) | HotkeyBindStatus::Error(msg) => {
                ui.add_space(4.0);
                ui.colored_label(STATUS_ERR, RichText::new(msg).size(12.0));
            }
            HotkeyBindStatus::Disabled | HotkeyBindStatus::Bound(_) => {}
        }
        ui.weak("Example: Ctrl+Shift+O, Alt+F12, Ctrl+Alt+H");
    });
}
