use crate::theme::{STATUS_OK, STATUS_WARN, card_frame, section_title, status_badge};
use crate::worker::{SharedState, WorkerCommand};
use egui::{ScrollArea, Ui};
use omvix_core::{CustomThemeConfig, I18n};
use std::path::PathBuf;

pub fn render(
    ui: &mut Ui,
    st: &SharedState,
    i18n: &I18n,
    t: &CustomThemeConfig,
    on_cmd: &mut dyn FnMut(WorkerCommand),
) {
    ui.horizontal(|ui| {
        ui.heading(i18n.t("diagnostics.title"));
        ui.add_space(12.0);
        if ui.button(i18n.t("diagnostics.run_probe")).clicked() {
            on_cmd(WorkerCommand::RefreshDiagnostics);
        }
        if ui.button(i18n.t("diagnostics.export_json")).clicked() {
            let out_path = default_export_path();
            on_cmd(WorkerCommand::ExportDiagnostics(out_path));
        }
    });
    ui.add_space(6.0);

    if let Some(diag) = &st.diagnostics {
        // System Identity & Board Match
        card_frame(t).show(ui, |ui| {
            section_title(ui, i18n.t("diagnostics.hardware_firmware"), t);
            egui::Grid::new("diag_sys_grid")
                .num_columns(2)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Model");
                    ui.label(format!("{} ({})", diag.identity.model, diag.identity.system_family));
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
                            b.marketing_name, b.thermal_scheme, b.expected_has_mux, b.expected_keyboard
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
                        ui.label(format!("{} ({}) — {}", svc.service_name, svc.category, svc.display_name));
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
