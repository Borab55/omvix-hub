use crate::theme::{ACCENT_CYAN, card_frame};
use crate::worker::WorkerCommand;
use egui::{RichText, ScrollArea, Slider, Ui};
use omvix_core::{
    AppConfig, I18n, LogLevel, clear_logs, tail_logs, total_log_bytes,
};

#[cfg(windows)]
use omvix_windows::autostart;

pub struct SettingsActions {
    pub language_changed: bool,
    pub theme_changed: bool,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &mut I18n,
    dark: bool,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) -> SettingsActions {
    let mut actions = SettingsActions {
        language_changed: false,
        theme_changed: false,
    };

    ui.heading(i18n.t("nav.settings"));
    ui.add_space(6.0);

    // General, Language, Theme, Tray, Autostart
    card_frame(dark).show(ui, |ui| {
        ui.label(RichText::new("General").strong().color(ACCENT_CYAN));
        ui.horizontal(|ui| {
            ui.label(i18n.t("common.language"));
            let langs = i18n.available_languages();
            let current_label = langs
                .iter()
                .find(|l| l.code == cfg.general.language)
                .map(|l| l.display_name.clone())
                .unwrap_or_else(|| cfg.general.language.clone());
            egui::ComboBox::from_id_salt("lang_combo")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for lang in langs {
                        if ui
                            .selectable_value(
                                &mut cfg.general.language,
                                lang.code.clone(),
                                lang.display_name,
                            )
                            .changed()
                        {
                            i18n.set_language(&cfg.general.language);
                            on_save();
                            actions.language_changed = true;
                        }
                    }
                });

            ui.add_space(16.0);
            ui.label(i18n.t("settings.theme"));
            if ui
                .selectable_label(cfg.general.theme == "dark", i18n.t("settings.theme_dark"))
                .clicked()
            {
                cfg.general.theme = "dark".to_string();
                on_save();
                actions.theme_changed = true;
            }
            if ui
                .selectable_label(cfg.general.theme == "light", i18n.t("settings.theme_light"))
                .clicked()
            {
                cfg.general.theme = "light".to_string();
                on_save();
                actions.theme_changed = true;
            }
        });

        if ui
            .checkbox(
                &mut cfg.general.minimize_to_tray_on_close,
                i18n.t("common.minimize_to_tray_on_close"),
            )
            .changed()
        {
            on_save();
        }
        if ui
            .checkbox(
                &mut cfg.general.start_minimized,
                i18n.t("settings.start_minimized"),
            )
            .changed()
        {
            on_save();
        }

        if ui
            .checkbox(
                &mut cfg.general.start_with_windows,
                i18n.t("common.start_with_windows"),
            )
            .changed()
        {
            #[cfg(windows)]
            if let Ok(exe) = std::env::current_exe() {
                if let Err(err) = autostart::set_autostart(cfg.general.start_with_windows, &exe) {
                    tracing::warn!("Autostart update failed: {err}");
                }
            }
            on_save();
        }
    });

    ui.add_space(8.0);

    // Diagnostic Rotating Logging Card
    card_frame(dark).show(ui, |ui| {
        ui.label(RichText::new(i18n.t("logs.title")).strong().color(ACCENT_CYAN));
        let log_dir = cfg.logging.resolved_dir();
        let bytes = total_log_bytes(&log_dir);
        ui.label(format!(
            "{}: {:.2} MB / {} MB",
            i18n.t("logs.current_usage"),
            bytes as f64 / (1024.0 * 1024.0),
            cfg.logging.max_total_mb
        ));

        ui.horizontal(|ui| {
            ui.label(i18n.t("logs.level"));
            egui::ComboBox::from_id_salt("log_level_combo")
                .selected_text(format!("{:?}", cfg.logging.level))
                .show_ui(ui, |ui| {
                    for &lvl in LogLevel::all() {
                        if ui
                            .selectable_value(&mut cfg.logging.level, lvl, format!("{lvl:?}"))
                            .changed()
                        {
                            on_save();
                            on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        }
                    }
                });

            ui.add_space(12.0);
            ui.label(i18n.t("logs.max_size_mb"));
            if ui
                .add(Slider::new(&mut cfg.logging.max_total_mb, 5..=100).suffix(" MB"))
                .changed()
            {
                on_save();
            }
        });

        ui.horizontal(|ui| {
            if ui.button(i18n.t("logs.open_folder")).clicked() {
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(&log_dir)
                        .spawn();
                }
            }
            if ui.button(i18n.t("logs.clear_logs")).clicked() {
                let _ = clear_logs(&log_dir);
            }
        });

        ui.collapsing(i18n.t("logs.preview"), |ui| {
            let lines = tail_logs(&log_dir, 30);
            ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                for line in lines {
                    ui.monospace(line);
                }
            });
        });
    });

    actions
}
