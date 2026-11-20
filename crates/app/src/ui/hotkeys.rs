use crate::theme::{ACCENT_CYAN, STATUS_ERR, STATUS_OK, card_frame, status_badge};
use crate::worker::WorkerCommand;
use egui::{RichText, Ui};
use omvix_core::config::OmenKeyAction;
use omvix_core::{AppConfig, I18n};

#[cfg(windows)]
use omvix_windows::hotkeys::parse_shortcut;

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &I18n,
    dark: bool,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
    on_rebind_hotkeys: &mut dyn FnMut(),
) {
    ui.heading(i18n.t("nav.hotkeys"));
    ui.add_space(6.0);

    // Omen Key Card
    card_frame(dark).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(i18n.t("hotkeys.omen_key")).strong().color(ACCENT_CYAN));
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
                ui.horizontal(|ui| {
                    ui.label(i18n.t("hotkeys.custom_command"));
                    if ui
                        .text_edit_singleline(&mut cfg.hotkeys.custom_command)
                        .changed()
                    {
                        on_save();
                    }
                });
            }
        });
    });

    ui.add_space(8.0);

    // Global Shortcut Card
    card_frame(dark).show(ui, |ui| {
        ui.label(RichText::new(i18n.t("hotkeys.global_shortcut")).strong().color(ACCENT_CYAN));
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut cfg.hotkeys.global_toggle_shortcut);

            #[cfg(windows)]
            {
                let valid = parse_shortcut(&cfg.hotkeys.global_toggle_shortcut).is_ok();
                if valid {
                    status_badge(ui, &i18n.t("common.status_ok"), STATUS_OK);
                } else {
                    status_badge(ui, &i18n.t("common.status_error"), STATUS_ERR);
                }
            }

            if ui.button(i18n.t("common.apply")).clicked() {
                on_save();
                on_rebind_hotkeys();
            }
        });
        ui.weak("Example: Ctrl+Shift+O, Alt+F12, Ctrl+Alt+H");
    });
}
