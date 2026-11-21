use crate::theme::{STATUS_ERR, STATUS_OK, STATUS_WARN, card_frame, pill_button, section_title, status_badge};
use crate::ui::control_committed;
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, ScrollArea, Slider, Ui};
use omvix_core::{
    AppConfig, CustomThemeConfig, I18n, LogLevel, clear_logs, tail_logs, total_log_bytes,
};

#[derive(Debug, Default, Clone)]
pub struct SettingsThemeDraft {
    pub import_input: String,
    pub status_msg: Option<(String, bool)>,
    pub confirm_reset: bool,
}

pub struct SettingsActions {
    pub language_changed: bool,
    pub theme_changed: bool,
    pub dock_changed: bool,
    pub restore_defaults: bool,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &mut I18n,
    draft: &mut SettingsThemeDraft,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) -> SettingsActions {
    let mut actions = SettingsActions {
        language_changed: false,
        theme_changed: false,
        dock_changed: false,
        restore_defaults: false,
    };
    let theme_snapshot = cfg.general.custom_theme.clone();

    ui.heading(i18n.t("nav.settings"));
    ui.add_space(6.0);

    // General, Language, Refresh Rate, Tray, Autostart
    card_frame(&theme_snapshot).show(ui, |ui| {
        section_title(ui, i18n.t("settings.general"), &theme_snapshot);
        ui.horizontal_wrapped(|ui| {
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

            ui.add_space(12.0);
            ui.label(i18n.t("monitor.refresh_rate"));
            for &(ms, label) in &[
                (100u64, "0.1s"),
                (250, "0.25s"),
                (500, "0.5s"),
                (1000, "1s"),
                (1500, "1.5s"),
                (2000, "2s"),
                (5000, "5s"),
            ] {
                if ui
                    .selectable_label(cfg.general.telemetry_interval_ms == ms, label)
                    .clicked()
                {
                    cfg.general.telemetry_interval_ms = ms;
                    on_save();
                    on_cmd(WorkerCommand::SetTelemetryIntervalMs(ms));
                }
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
                &mut cfg.general.dock_bottom_right,
                i18n.t("settings.dock_bottom_right"),
            )
            .changed()
        {
            on_save();
            actions.dock_changed = true;
        }
        if ui
            .checkbox(
                &mut cfg.general.hide_on_focus_loss,
                i18n.t("settings.hide_on_focus_loss"),
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

        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(
                    &mut cfg.general.start_with_windows,
                    i18n.t("common.start_with_windows"),
                )
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::SetAutostart(cfg.general.start_with_windows));
            }
            if st.installed_copy_status.task_exists {
                status_badge(ui, &i18n.t("settings.autostart_elevated_badge"), STATUS_OK);
            }
        });
        ui.weak(RichText::new(i18n.t("settings.autostart_admin_note")).size(11.0));
        if !st.installed_copy_status.installed_path.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}:", i18n.t("settings.installed_copy_path")));
                ui.monospace(&st.installed_copy_status.installed_path);
                if pill_button(
                    ui,
                    &i18n.t("settings.update_installed_copy"),
                    st.installed_copy_status.installed_is_outdated,
                    &theme_snapshot,
                )
                .clicked()
                {
                    on_cmd(WorkerCommand::UpdateInstalledCopy);
                }
            });
            if st.installed_copy_status.installed_is_outdated {
                ui.colored_label(
                    STATUS_WARN,
                    RichText::new(i18n.t("settings.installed_copy_outdated")).size(11.5),
                );
            }
        }
    });

    ui.add_space(8.0);

    // Custom Theme Colors & Copy/Paste Profile Sharing Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        section_title(ui, i18n.t("settings.custom_theme"), &theme_snapshot);
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.theme_presets"));
            for preset in CustomThemeConfig::presets() {
                let selected = cfg.general.custom_theme == preset;
                if ui.selectable_label(selected, &preset.name).clicked() {
                    cfg.general.custom_theme = preset.clone();
                    cfg.general.theme = if preset.dark_base {
                        "dark".to_string()
                    } else {
                        "light".to_string()
                    };
                    on_save();
                    actions.theme_changed = true;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.theme_name"));
            if ui
                .text_edit_singleline(&mut cfg.general.custom_theme.name)
                .changed()
            {
                on_save();
            }
            if ui
                .checkbox(
                    &mut cfg.general.custom_theme.dark_base,
                    i18n.t("settings.theme_dark_base"),
                )
                .changed()
            {
                cfg.general.theme = if cfg.general.custom_theme.dark_base {
                    "dark".to_string()
                } else {
                    "light".to_string()
                };
                on_save();
                actions.theme_changed = true;
            }
        });

        ui.add_space(4.0);
        egui::Grid::new("custom_theme_pickers_grid")
            .num_columns(6)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label(i18n.t("settings.color_accent"));
                let r0 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.accent);
                if r0.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 0usize), &r0) {
                    on_save();
                }

                ui.label(i18n.t("settings.color_canvas"));
                let r1 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.canvas_bg);
                if r1.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 1usize), &r1) {
                    on_save();
                }

                ui.label(i18n.t("settings.color_sidebar"));
                let r2 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.sidebar_bg);
                if r2.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 2usize), &r2) {
                    on_save();
                }
                ui.end_row();

                ui.label(i18n.t("settings.color_card"));
                let r3 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.card_bg);
                if r3.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 3usize), &r3) {
                    on_save();
                }

                ui.label(i18n.t("settings.color_border"));
                let r4 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.card_border);
                if r4.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 4usize), &r4) {
                    on_save();
                }

                ui.label(i18n.t("settings.color_text"));
                let r5 = ui.color_edit_button_srgb(&mut cfg.general.custom_theme.text_primary);
                if r5.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, ("theme_color", 5usize), &r5) {
                    on_save();
                }
                ui.end_row();
            });

        ui.separator();

        let share_code = cfg.general.custom_theme.encode_share_code();
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.share_code"));
            let mut preview = share_code.clone();
            ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut preview).desired_width(360.0),
            );
            if ui.button(i18n.t("settings.copy_theme")).clicked() {
                ui.ctx().copy_text(share_code);
                draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
            }
            if ui.button(i18n.t("settings.copy_theme_json")).clicked() {
                if let Ok(json) = serde_json::to_string_pretty(&cfg.general.custom_theme) {
                    ui.ctx().copy_text(json);
                    draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
                }
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.import_input)
                    .hint_text(i18n.t("settings.import_placeholder"))
                    .desired_width(360.0),
            );
            if ui
                .button(RichText::new(i18n.t("settings.import_theme")).strong())
                .clicked()
            {
                match CustomThemeConfig::decode_share_code(&draft.import_input) {
                    Ok(imported) => {
                        cfg.general.custom_theme = imported.clone();
                        cfg.general.theme = if imported.dark_base {
                            "dark".to_string()
                        } else {
                            "light".to_string()
                        };
                        on_save();
                        actions.theme_changed = true;
                        draft.import_input.clear();
                        draft.status_msg = Some((i18n.t("settings.theme_imported"), false));
                    }
                    Err(err) => {
                        draft.status_msg = Some((err.to_string(), true));
                    }
                }
            }
        });

        if let Some((msg, is_err)) = &draft.status_msg {
            ui.colored_label(if *is_err { STATUS_ERR } else { STATUS_OK }, msg);
        }
    });

    ui.add_space(8.0);

    // Diagnostic Rotating Logging Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        section_title(ui, i18n.t("logs.title"), &theme_snapshot);
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
            let log_resp =
                ui.add(Slider::new(&mut cfg.logging.max_total_mb, 5..=100).suffix(" MB"));
            if control_committed(ui, "settings_log_max_mb", &log_resp) {
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

    ui.add_space(8.0);

    // PawnIO CPU Die Temperature Sensor Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, i18n.t("settings.pawnio_section"), &theme_snapshot);
            if st.pawnio_cpu_active {
                status_badge(ui, &i18n.t("monitor.pawnio_active"), STATUS_OK);
            } else {
                status_badge(ui, &i18n.t("common.disabled"), STATUS_WARN);
            }
        });
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.pawnio_desc"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if st.pawnio_installing {
                ui.weak(i18n.t("monitor.installing_pawnio"));
            } else if pill_button(
                ui,
                &i18n.t("monitor.install_pawnio"),
                !st.pawnio_cpu_active,
                &theme_snapshot,
            )
            .clicked()
            {
                on_cmd(WorkerCommand::InstallPawnIo);
            }
        });
    });

    ui.add_space(8.0);

    // Safety & Factory Defaults Reset Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        section_title(ui, i18n.t("settings.reset_section"), &theme_snapshot);
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.reset_desc"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if !draft.confirm_reset {
                if ui.button(i18n.t("settings.reset_all_btn")).clicked() {
                    draft.confirm_reset = true;
                }
            } else {
                if ui
                    .button(
                        RichText::new(i18n.t("settings.reset_confirm_btn"))
                            .strong()
                            .color(STATUS_ERR),
                    )
                    .clicked()
                {
                    draft.confirm_reset = false;
                    actions.restore_defaults = true;
                }
                if ui.button(i18n.t("common.cancel")).clicked() {
                    draft.confirm_reset = false;
                }
            }
        });
    });

    actions
}
