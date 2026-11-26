use crate::theme::{
    accent_color, card_frame, feature_toggle_button, pill_button, section_title, status_badge,
    status_err, status_ok, status_warn,
};
use crate::ui::{
    control_committed, default_support_bundle_path, open_folder_in_explorer, show_file_in_folder,
    REFRESH_OPTIONS_MS,
};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, ScrollArea, Slider, Ui};
use omvix_core::updater::GITHUB_RELEASES_PAGE_URL;
use omvix_core::{
    clear_logs, tail_logs, total_log_bytes, AppConfig, I18n, LogLevel, UpdateSignatureStatus,
    UpdateUiStatus,
};

#[derive(Debug, Default, Clone)]
pub struct SettingsThemeDraft {
    pub confirm_reset: bool,
    pub scroll_to_updates: bool,
}

pub struct SettingsActions {
    pub language_changed: bool,
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
            for &(ms, label) in REFRESH_OPTIONS_MS {
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
                open_folder_in_explorer(&log_dir);
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
    let upd_resp = card_frame(&theme_snapshot)
        .show(ui, |ui| {
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
        })
        .response;
    if draft.scroll_to_updates {
        upd_resp.scroll_to_me(Some(egui::Align::TOP));
        draft.scroll_to_updates = false;
    }

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
