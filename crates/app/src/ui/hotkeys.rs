use crate::theme::{STATUS_ERR, STATUS_OK, STATUS_WARN, card_frame, section_title, status_badge};
use crate::worker::WorkerCommand;
use egui::{RichText, Ui};
use omvix_core::config::{OmenKeyAction, is_valid_custom_command_syntax};
use omvix_core::{AppConfig, I18n};

#[cfg(windows)]
use omvix_windows::hotkeys::parse_shortcut;

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &I18n,
    config_warnings: &mut Vec<String>,
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
                        cfg.hotkeys.custom_args.retain(|c| !matches!(c, '\0' | '\r' | '\n'));
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

            let enter_applied =
                shortcut_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid;

            if ui.button(i18n.t("common.apply")).clicked() || enter_applied {
                on_save();
                on_rebind_hotkeys();
            }
        });
        ui.weak("Example: Ctrl+Shift+O, Alt+F12, Ctrl+Alt+H");
    });
}
