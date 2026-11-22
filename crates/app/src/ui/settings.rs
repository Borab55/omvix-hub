use crate::theme::{
    accent_color, card_frame, feature_toggle_button, fixed_color_picker_button, pill_button, rgb,
    saved_theme_chip, section_title, status_badge, status_err, status_ok, status_warn,
    ColorButtonStyle, SavedChipAction,
};
use crate::ui::{control_committed, default_support_bundle_path, show_file_in_folder};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, ScrollArea, Slider, Ui};
use omvix_core::updater::GITHUB_RELEASES_PAGE_URL;
use omvix_core::{
    clear_logs, tail_logs, total_log_bytes, AppConfig, CustomThemeConfig, I18n, LogLevel,
    UpdateSignatureStatus, UpdateUiStatus,
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
    pub open_wizard: bool,
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
        open_wizard: false,
        restore_defaults: false,
    };
    let theme_snapshot = cfg.general.custom_theme.clone();
    let ok_c = status_ok(&theme_snapshot);
    let warn_c = status_warn(&theme_snapshot);
    let err_c = status_err(&theme_snapshot);

    ui.heading(i18n.t("nav.settings"));
    ui.add_space(6.0);

    // General, Language, Refresh Rate, Tray, Autostart
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
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

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("common.minimize_to_tray_on_close"),
                &mut cfg.general.minimize_to_tray_on_close,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
            }
            if feature_toggle_button(
                ui,
                &i18n.t("settings.dock_bottom_right"),
                &mut cfg.general.dock_bottom_right,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
                actions.dock_changed = true;
            }
            if feature_toggle_button(
                ui,
                &i18n.t("settings.hide_on_focus_loss"),
                &mut cfg.general.hide_on_focus_loss,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
            }
            if feature_toggle_button(
                ui,
                &i18n.t("settings.start_minimized"),
                &mut cfg.general.start_minimized,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
            }
        });
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("common.start_with_windows"),
                &mut cfg.general.start_with_windows,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::SetAutostart(cfg.general.start_with_windows));
            }
            if st.installed_copy_status.task_exists {
                status_badge(ui, &i18n.t("settings.autostart_elevated_badge"), ok_c);
            }
        });
        ui.weak(RichText::new(i18n.t("settings.autostart_admin_note")).size(11.0));
        if !st.installed_copy_status.installed_path.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}:", i18n.t("settings.installed_copy_path")));
                ui.add(
                    egui::Label::new(
                        RichText::new(&st.installed_copy_status.installed_path).monospace(),
                    )
                    .wrap(),
                );
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
                    warn_c,
                    RichText::new(i18n.t("settings.installed_copy_outdated")).size(11.5),
                );
            }
        }
        ui.add_space(6.0);
        if ui.button(i18n.t("settings.open_wizard")).clicked() {
            actions.open_wizard = true;
        }
    });

    ui.add_space(8.0);

    // Accessibility Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("settings.accessibility"), &theme_snapshot);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.font_scale"));
            let scale_resp = ui.add(
                egui::Slider::new(&mut cfg.general.font_scale_pct, 85..=130)
                    .suffix("%")
                    .step_by(5.0),
            );
            if control_committed(ui, "settings_font_scale", &scale_resp) {
                actions.theme_changed = true;
                on_save();
            }
            for pct in [85u8, 100, 115, 130] {
                if ui
                    .selectable_label(cfg.general.font_scale_pct == pct, format!("{pct}%"))
                    .clicked()
                {
                    cfg.general.font_scale_pct = pct;
                    actions.theme_changed = true;
                    on_save();
                }
            }
        });
        ui.add_space(4.0);
        let mut high_contrast = cfg.general.custom_theme.is_high_contrast();
        if feature_toggle_button(
            ui,
            &i18n.t("settings.high_contrast_mode"),
            &mut high_contrast,
            &i18n.t("common.enabled"),
            &theme_snapshot,
        )
        .changed()
        {
            cfg.general.custom_theme = if high_contrast {
                CustomThemeConfig::preset_high_contrast()
            } else {
                CustomThemeConfig::preset_dark_slate()
            };
            actions.theme_changed = true;
            on_save();
        }
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.keyboard_nav_hint"));
    });

    ui.add_space(8.0);

    // Custom Theme Colors & Copy/Paste Profile Sharing Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("settings.custom_theme"), &theme_snapshot);
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.theme_presets"));
            for preset in CustomThemeConfig::presets() {
                let selected = cfg.general.custom_theme == preset;
                if pill_button(ui, &preset.name, selected, &theme_snapshot).clicked() {
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
                .add(
                    egui::TextEdit::singleline(&mut cfg.general.custom_theme.name)
                        .desired_width((card_w * 0.34).clamp(130.0, 180.0))
                        .clip_text(true),
                )
                .changed()
            {
                on_save();
            }
            if pill_button(
                ui,
                &i18n.t("settings.save_theme_btn"),
                false,
                &theme_snapshot,
            )
            .clicked()
            {
                let mut to_save = cfg.general.custom_theme.clone();
                let clean = to_save.name.trim();
                to_save.name = if clean.is_empty() {
                    format!("Theme {}", cfg.general.custom_themes.len() + 1)
                } else {
                    clean.to_string()
                };
                if let Some(existing) = cfg
                    .general
                    .custom_themes
                    .iter_mut()
                    .find(|x| x.name.eq_ignore_ascii_case(&to_save.name))
                {
                    *existing = to_save;
                } else {
                    cfg.general.custom_themes.push(to_save);
                }
                on_save();
                draft.status_msg = Some((i18n.t("settings.theme_saved"), false));
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("settings.theme_dark_base"),
                &mut cfg.general.custom_theme.dark_base,
                &i18n.t("common.enabled"),
                &theme_snapshot,
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
            .num_columns(4)
            .min_col_width(0.0)
            .max_col_width(((card_w - 110.0) * 0.5).max(90.0))
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                let swatch_style = ColorButtonStyle::Swatch {
                    size: egui::vec2(34.0, 20.0),
                    radius: 5,
                };
                let color_fields: [(&str, &mut [u8; 3]); 12] = [
                    (
                        "settings.color_accent",
                        &mut cfg.general.custom_theme.accent,
                    ),
                    (
                        "settings.color_sidebar",
                        &mut cfg.general.custom_theme.sidebar_bg,
                    ),
                    (
                        "settings.color_canvas",
                        &mut cfg.general.custom_theme.canvas_bg,
                    ),
                    ("settings.color_card", &mut cfg.general.custom_theme.card_bg),
                    (
                        "settings.color_border",
                        &mut cfg.general.custom_theme.card_border,
                    ),
                    (
                        "settings.color_text",
                        &mut cfg.general.custom_theme.text_primary,
                    ),
                    (
                        "settings.color_text_muted",
                        &mut cfg.general.custom_theme.text_muted,
                    ),
                    (
                        "settings.color_control_bg",
                        &mut cfg.general.custom_theme.control_bg,
                    ),
                    (
                        "settings.color_control_hover",
                        &mut cfg.general.custom_theme.control_hover_bg,
                    ),
                    (
                        "settings.color_status_ok",
                        &mut cfg.general.custom_theme.status_ok,
                    ),
                    (
                        "settings.color_status_warn",
                        &mut cfg.general.custom_theme.status_warn,
                    ),
                    (
                        "settings.color_status_err",
                        &mut cfg.general.custom_theme.status_err,
                    ),
                ];
                for (idx, (label_key, field)) in color_fields.into_iter().enumerate() {
                    ui.label(i18n.t(label_key));
                    let resp = fixed_color_picker_button(
                        ui,
                        ("settings_theme_color_picker", idx),
                        field,
                        swatch_style,
                        &theme_snapshot,
                    );
                    if resp.changed() {
                        actions.theme_changed = true;
                    }
                    if control_committed(ui, ("theme_color", idx), &resp) {
                        on_save();
                    }
                    if idx % 2 == 1 {
                        ui.end_row();
                    }
                }
            });

        ui.add_space(6.0);
        section_title(ui, i18n.t("settings.saved_themes"), &theme_snapshot);
        ui.add_space(4.0);
        if cfg.general.custom_themes.is_empty() {
            ui.weak(i18n.t("settings.no_saved_themes"));
        } else {
            let mut apply_saved: Option<CustomThemeConfig> = None;
            let mut delete_saved: Option<usize> = None;
            let del_tip = i18n.t("settings.delete_theme_tooltip");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for (idx, saved) in cfg.general.custom_themes.iter().enumerate() {
                    let active = cfg.general.custom_theme == *saved;
                    let chip_action = ui
                        .push_id(idx, |ui| {
                            saved_theme_chip(
                                ui,
                                &saved.name,
                                rgb(saved.accent),
                                active,
                                &del_tip,
                                &theme_snapshot,
                            )
                        })
                        .inner;
                    match chip_action {
                        SavedChipAction::Apply => apply_saved = Some(saved.clone()),
                        SavedChipAction::Delete => delete_saved = Some(idx),
                        SavedChipAction::None => {}
                    }
                }
            });
            if let Some(chosen) = apply_saved {
                cfg.general.custom_theme = chosen.clone();
                cfg.general.theme = if chosen.dark_base {
                    "dark".to_string()
                } else {
                    "light".to_string()
                };
                on_save();
                actions.theme_changed = true;
            }
            if let Some(idx) = delete_saved {
                cfg.general.custom_themes.remove(idx);
                on_save();
            }
        }

        ui.separator();

        let full_input_w = (card_w - 8.0).max(160.0);
        let share_code = cfg.general.custom_theme.encode_share_code();
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.share_code"));
            if ui.button(i18n.t("settings.copy_theme")).clicked() {
                ui.ctx().copy_text(share_code.clone());
                draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
            }
            if ui.button(i18n.t("settings.copy_theme_json")).clicked() {
                if let Ok(json) = serde_json::to_string_pretty(&cfg.general.custom_theme) {
                    ui.ctx().copy_text(json);
                    draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
                }
            }
        });
        let mut preview = share_code;
        ui.add_enabled(
            false,
            egui::TextEdit::singleline(&mut preview)
                .desired_width(full_input_w)
                .clip_text(true),
        );

        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut draft.import_input)
                .hint_text(i18n.t("settings.import_placeholder"))
                .desired_width(full_input_w)
                .clip_text(true),
        );
        ui.horizontal_wrapped(|ui| {
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
            ui.colored_label(if *is_err { err_c } else { ok_c }, msg);
        }
    });

    ui.add_space(8.0);

    // Diagnostic Rotating Logging Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("logs.title"), &theme_snapshot);
        let log_dir = cfg.logging.resolved_dir();
        let bytes = total_log_bytes(&log_dir);
        ui.label(format!(
            "{}: {:.2} MB / {} MB",
            i18n.t("logs.current_usage"),
            bytes as f64 / (1024.0 * 1024.0),
            cfg.logging.max_total_mb
        ));

        ui.horizontal_wrapped(|ui| {
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

        ui.horizontal_wrapped(|ui| {
            if ui.button(i18n.t("logs.open_folder")).clicked() {
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new(omvix_windows::paths::system_tool(
                        "explorer.exe",
                    ))
                    .arg(&log_dir)
                    .spawn();
                }
            }
            if ui.button(i18n.t("logs.clear_logs")).clicked() {
                let _ = clear_logs(&log_dir);
            }
            let bundle_lbl = if st.support_bundle_busy {
                i18n.t("support_bundle.creating")
            } else {
                i18n.t("support_bundle.create_btn")
            };
            if ui
                .add_enabled(!st.support_bundle_busy, egui::Button::new(bundle_lbl))
                .clicked()
            {
                on_cmd(WorkerCommand::CreateSupportBundle(
                    default_support_bundle_path(),
                ));
            }
        });

        if let Some(ref zip_path) = st.last_support_bundle_path {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{}: {}",
                            i18n.t("support_bundle.saved_label"),
                            zip_path.display()
                        ))
                        .weak(),
                    )
                    .wrap(),
                );
                if ui
                    .small_button(i18n.t("support_bundle.show_in_folder"))
                    .clicked()
                {
                    show_file_in_folder(zip_path);
                }
            });
        }

        ui.collapsing(i18n.t("logs.preview"), |ui| {
            let lines = tail_logs(&log_dir, 30);
            ScrollArea::both()
                .id_salt("settings_log_preview_scroll")
                .max_width(ui.available_width())
                .max_height(160.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in lines {
                        ui.monospace(line);
                    }
                });
        });
    });

    ui.add_space(8.0);

    // Software Updates Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("updates.title"), &theme_snapshot);
            status_badge(
                ui,
                &format!("v{}", env!("CARGO_PKG_VERSION")),
                accent_color(&theme_snapshot),
            );
            match &st.update_status {
                UpdateUiStatus::Idle => {}
                UpdateUiStatus::Checking => {
                    status_badge(ui, &i18n.t("updates.checking"), warn_c);
                }
                UpdateUiStatus::UpToDate { .. } => {
                    status_badge(ui, &i18n.t("updates.up_to_date"), ok_c);
                }
                UpdateUiStatus::NoReleasesPublished => {
                    status_badge(
                        ui,
                        &i18n.t("updates.no_releases"),
                        ui.visuals().weak_text_color(),
                    );
                }
                UpdateUiStatus::UpdateAvailable(_) => {
                    status_badge(ui, &i18n.t("updates.available_badge"), ok_c);
                }
                UpdateUiStatus::Downloading { .. } => {
                    status_badge(ui, &i18n.t("updates.downloading"), warn_c);
                }
                UpdateUiStatus::ReadyToInstall(_) => {
                    status_badge(ui, &i18n.t("updates.install_restart_btn"), ok_c);
                }
                UpdateUiStatus::Applying { .. } => {
                    status_badge(ui, &i18n.t("updates.applying"), warn_c);
                }
                UpdateUiStatus::Error(_) => {
                    status_badge(ui, &i18n.t("common.status_error"), err_c);
                }
            }
        });
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("updates.auto_check"),
                &mut cfg.general.auto_check_updates,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            if feature_toggle_button(
                ui,
                &i18n.t("updates.allow_unsigned"),
                &mut cfg.general.allow_unsigned_sha256_updates,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });
        ui.weak(RichText::new(i18n.t("updates.signature_policy_note")).size(11.0));
        ui.add_space(6.0);

        let update_busy = matches!(
            st.update_status,
            UpdateUiStatus::Checking
                | UpdateUiStatus::Downloading { .. }
                | UpdateUiStatus::Applying { .. }
        );
        let release_page_url = match &st.update_status {
            UpdateUiStatus::UpdateAvailable(rel) => rel.html_url.clone(),
            UpdateUiStatus::ReadyToInstall(staged) => staged.release_url.clone(),
            _ => GITHUB_RELEASES_PAGE_URL.to_string(),
        };

        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!update_busy, egui::Button::new(i18n.t("updates.check_btn")))
                .clicked()
            {
                on_cmd(WorkerCommand::CheckForUpdates { silent: false });
            }
            if ui.button(i18n.t("updates.open_github_btn")).clicked() {
                on_cmd(WorkerCommand::OpenReleasePage(release_page_url));
            }
        });

        match &st.update_status {
            UpdateUiStatus::UpToDate {
                checked_version,
                checked_at_utc,
            } => {
                ui.add_space(4.0);
                ui.weak(format!(
                    "{} ({checked_version}, {checked_at_utc})",
                    i18n.t("updates.up_to_date")
                ));
            }
            UpdateUiStatus::NoReleasesPublished => {
                ui.add_space(4.0);
                ui.weak(i18n.t("updates.no_releases"));
            }
            UpdateUiStatus::UpdateAvailable(rel) => {
                ui.add_space(6.0);
                let size_mb = rel.binary_asset.size_bytes as f64 / (1024.0 * 1024.0);
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("v{}", rel.version));
                    if !rel.published_at.is_empty() {
                        ui.weak(&rel.published_at);
                    }
                    ui.weak(format!("• {} ({size_mb:.2} MB)", rel.binary_asset.name));
                    if rel.checksum_asset.is_some() {
                        status_badge(ui, &i18n.t("updates.checksum_ok"), ok_c);
                    } else {
                        status_badge(ui, &i18n.t("updates.checksum_missing"), warn_c);
                    }
                });
                if !rel.notes.trim().is_empty() {
                    ui.collapsing(i18n.t("updates.release_notes"), |ui| {
                        ScrollArea::vertical()
                            .max_width(ui.available_width())
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.label(&rel.notes);
                            });
                    });
                }
                ui.add_space(4.0);
                ui.add_enabled_ui(rel.checksum_asset.is_some(), |ui| {
                    if pill_button(
                        ui,
                        &i18n.t("updates.download_verify_btn"),
                        rel.checksum_asset.is_some(),
                        &theme_snapshot,
                    )
                    .clicked()
                    {
                        on_cmd(WorkerCommand::DownloadAndVerifyUpdate);
                    }
                });
            }
            UpdateUiStatus::ReadyToInstall(staged) => {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("SHA-256: {}", staged.verified_sha256))
                            .monospace()
                            .size(11.5),
                    )
                    .wrap(),
                );
                ui.horizontal_wrapped(|ui| match &staged.signature {
                    UpdateSignatureStatus::AuthenticodeValid {
                        subject,
                        thumbprint,
                        ..
                    } => {
                        status_badge(ui, &i18n.t("updates.sig_valid"), ok_c);
                        ui.weak(format!("{subject} ({thumbprint})"));
                    }
                    UpdateSignatureStatus::UnsignedSha256Only => {
                        status_badge(ui, &i18n.t("updates.sig_unsigned"), warn_c);
                    }
                    UpdateSignatureStatus::Rejected(reason) => {
                        status_badge(ui, &i18n.t("updates.sig_rejected"), err_c);
                        ui.colored_label(err_c, reason);
                    }
                });
                ui.add_space(4.0);
                if staged.can_install(cfg.general.allow_unsigned_sha256_updates) {
                    if pill_button(
                        ui,
                        &i18n.t("updates.install_restart_btn"),
                        true,
                        &theme_snapshot,
                    )
                    .clicked()
                    {
                        on_cmd(WorkerCommand::ApplyStagedUpdate);
                    }
                } else {
                    ui.colored_label(
                        warn_c,
                        RichText::new(i18n.t("updates.blocked_by_sig_policy")).size(11.5),
                    );
                }
            }
            UpdateUiStatus::Error(msg) => {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(msg).color(err_c)).wrap());
            }
            UpdateUiStatus::Idle
            | UpdateUiStatus::Checking
            | UpdateUiStatus::Downloading { .. }
            | UpdateUiStatus::Applying { .. } => {}
        }
    });

    ui.add_space(8.0);

    // PawnIO CPU Die Temperature Sensor Card
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("settings.pawnio_section"), &theme_snapshot);
            if st.pawnio_cpu_active {
                status_badge(ui, &i18n.t("monitor.pawnio_active"), ok_c);
            } else {
                status_badge(ui, &i18n.t("common.disabled"), warn_c);
            }
        });
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.pawnio_desc"));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
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
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("settings.reset_section"), &theme_snapshot);
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.reset_desc"));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if !draft.confirm_reset {
                if ui.button(i18n.t("settings.reset_all_btn")).clicked() {
                    draft.confirm_reset = true;
                }
            } else {
                if ui
                    .button(
                        RichText::new(i18n.t("settings.reset_confirm_btn"))
                            .strong()
                            .color(err_c),
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
