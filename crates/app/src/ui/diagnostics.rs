use crate::theme::{
    card_frame, pill_button_enabled, section_title, status_badge, STATUS_OK, STATUS_WARN,
};
use crate::ui::{default_support_bundle_path, show_file_in_folder};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, ScrollArea, Ui};
use omvix_core::{CustomThemeConfig, I18n};
use std::path::PathBuf;

pub fn render(
    ui: &mut Ui,
    st: &SharedState,
    i18n: &I18n,
    t: &CustomThemeConfig,
    on_cmd: &mut dyn FnMut(WorkerCommand),
) {
    ui.heading(i18n.t("diagnostics.title"));
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(i18n.t("diagnostics.run_probe")).clicked() {
            on_cmd(WorkerCommand::RefreshDiagnostics);
        }
        if ui.button(i18n.t("diagnostics.export_json")).clicked() {
            let out_path = default_export_path();
            on_cmd(WorkerCommand::ExportDiagnostics(out_path));
        }
        let bundle_lbl = if st.support_bundle_busy {
            i18n.t("support_bundle.creating")
        } else {
            i18n.t("support_bundle.create_btn")
        };
        if pill_button_enabled(ui, &bundle_lbl, true, !st.support_bundle_busy, t).clicked() {
            on_cmd(WorkerCommand::CreateSupportBundle(
                default_support_bundle_path(),
            ));
        }
    });
    ui.add_space(6.0);

    if let Some(ref zip_path) = st.last_support_bundle_path {
        card_frame(t).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, &i18n.t("support_bundle.saved_label"), STATUS_OK);
                ui.monospace(zip_path.display().to_string());
                if ui.button(i18n.t("support_bundle.show_in_folder")).clicked() {
                    show_file_in_folder(zip_path);
                }
            });
        });
        ui.add_space(6.0);
    }

    // Hardware Change History & 1-Click Undo Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("history.title"), t);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let has_entries = !st.change_history.is_empty();
                if ui
                    .add_enabled(has_entries, egui::Button::new(i18n.t("history.clear")))
                    .clicked()
                {
                    on_cmd(WorkerCommand::ClearChangeHistory);
                }
                if ui
                    .add_enabled(
                        has_entries,
                        egui::Button::new(i18n.t("history.undo_latest")),
                    )
                    .clicked()
                {
                    on_cmd(WorkerCommand::UndoLastChange);
                }
            });
        });

        if st.change_history.is_empty() {
            ui.weak(i18n.t("history.empty"));
        } else {
            ui.add_space(4.0);
            egui::Grid::new("change_history_grid")
                .striped(true)
                .num_columns(4)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    for entry in &st.change_history {
                        let ts_local = entry.timestamp.with_timezone(&chrono::Local);
                        ui.monospace(ts_local.format("%H:%M:%S").to_string());
                        ui.label(RichText::new(i18n.t(entry.domain.i18n_key())).strong());
                        ui.label(format!("{} -> {}", entry.old_summary, entry.new_summary));
                        if ui.small_button(i18n.t("history.undo_btn")).clicked() {
                            on_cmd(WorkerCommand::UndoChangeById(entry.id));
                        }
                        ui.end_row();
                    }
                });
        }
    });
    ui.add_space(8.0);

    if let Some(diag) = &st.diagnostics {
        // System Identity & Board Match
        card_frame(t).show(ui, |ui| {
            section_title(ui, i18n.t("diagnostics.hardware_firmware"), t);
            egui::Grid::new("diag_sys_grid")
                .num_columns(2)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Model");
                    ui.label(format!(
                        "{} ({})",
                        diag.identity.model, diag.identity.system_family
                    ));
                    ui.end_row();

                    ui.label(i18n.t("diagnostics.board_id"));
                    ui.label(&diag.identity.board_id);
                    ui.end_row();

                    ui.label(i18n.t("diagnostics.bios_version"));
                    ui.label(&diag.identity.bios_version);
                    ui.end_row();

                    ui.label("Board Profile");
                    if let Some(b) = &diag.matched_board {
                        ui.label(format!(
                            "{} | {:?} | MUX={} | Kbd={:?}",
                            b.marketing_name,
                            b.thermal_scheme,
                            b.expected_has_mux,
                            b.expected_keyboard
                        ));
                    } else {
                        ui.label("Unknown board (using safe fallback)");
                    }
                    ui.end_row();

                    ui.label("HP WMI Classes");
                    ui.label(format!(
                        "hpqBIntM={} | hpqBEvnt={}",
                        diag.wmi_hpqbintm_available, diag.wmi_hpqbevnt_available
                    ));
                    ui.end_row();
                });

            if let Some(err) = &diag.wmi_error {
                ui.add_space(4.0);
                ui.colored_label(STATUS_WARN, err);
            }
        });

        ui.add_space(8.0);

        // Conflict Services Card
        card_frame(t).show(ui, |ui| {
            section_title(ui, i18n.t("conflicts.title"), t);
            if diag.conflict_services.is_empty() {
                status_badge(ui, &i18n.t("conflicts.none_detected"), STATUS_OK);
            } else {
                for svc in &diag.conflict_services {
                    ui.horizontal(|ui| {
                        let color = if svc.status.eq_ignore_ascii_case("Running") {
                            STATUS_WARN
                        } else {
                            STATUS_OK
                        };
                        status_badge(ui, &svc.status, color);
                        ui.label(format!(
                            "{} ({}) — {}",
                            svc.service_name, svc.category, svc.display_name
                        ));
                    });
                }
            }
        });

        ui.add_space(8.0);

        // WMI Probe Table
        card_frame(t).show(ui, |ui| {
            section_title(
                ui,
                i18n.t_fmt(
                    "diagnostics.probe_table",
                    &[("count", &diag.probes.len().to_string())],
                ),
                t,
            );
            if diag.probes.is_empty() {
                ui.weak("Run Omvix Hub as Administrator to inspect live WMI BIOS responses.");
            } else {
                ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    egui::Grid::new("wmi_probe_grid")
                        .striped(true)
                        .num_columns(5)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.strong("Probe");
                            ui.strong("Cmd");
                            ui.strong("Type");
                            ui.strong("RC");
                            ui.strong("Data / Note");
                            ui.end_row();

                            for p in &diag.probes {
                                ui.label(&p.name);
                                ui.monospace(format!("0x{:X}", p.command));
                                ui.monospace(format!("0x{:X}", p.command_type));
                                ui.monospace(
                                    p.return_code
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "-".into()),
                                );
                                let detail = if p.success {
                                    p.data_hex.clone()
                                } else {
                                    p.error.clone().unwrap_or_else(|| "Failed".into())
                                };
                                ui.monospace(detail);
                                ui.end_row();
                            }
                        });
                });
            }
        });
    } else {
        ui.weak("Collecting hardware diagnostics...");
    }
}

fn default_export_path() -> PathBuf {
    #[cfg(windows)]
    if let Some(desktop) = std::env::var_os("USERPROFILE") {
        let candidate = PathBuf::from(desktop).join("Desktop");
        if candidate.exists() {
            return candidate.join("omvix-diagnostics.json");
        }
    }
    std::env::temp_dir().join("omvix-diagnostics.json")
}
