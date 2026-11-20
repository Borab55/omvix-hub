use crate::theme::{ACCENT_CYAN, STATUS_OK, STATUS_WARN, card_frame, status_badge};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use omvix_core::{AppConfig, I18n, PerformanceMode};

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    dark: bool,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_restart_admin: &mut dyn FnMut(),
) {
    ui.heading(i18n.t("dashboard.system_summary"));
    ui.add_space(6.0);

    // Non-admin banner or Admin active status
    if !st.security.is_admin {
        card_frame(dark).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, "STANDARD USER", STATUS_WARN);
                ui.label(i18n.t("app.non_admin_banner"));
                if ui
                    .button(RichText::new(i18n.t("app.restart_as_admin")).strong())
                    .clicked()
                {
                    on_restart_admin();
                }
            });
        });
        ui.add_space(8.0);
    } else {
        ui.horizontal(|ui| {
            status_badge(ui, &i18n.t("app.admin_active"), STATUS_OK);
            let running_conflicts = st
                .conflict_services
                .iter()
                .filter(|s| s.status.eq_ignore_ascii_case("Running"))
                .count();
            if running_conflicts > 0 {
                status_badge(
                    ui,
                    &i18n.t_fmt(
                        "conflicts.detected_summary",
                        &[("count", &running_conflicts.to_string())],
                    ),
                    STATUS_WARN,
                );
            }
        });
        ui.add_space(6.0);
    }

    // Quick performance mode selector bar
    card_frame(dark).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(i18n.t("thermal.mode")).strong());
            ui.add_space(8.0);
            for (mode, key) in [
                (PerformanceMode::Eco, "thermal.mode_eco"),
                (PerformanceMode::Quiet, "thermal.mode_quiet"),
                (PerformanceMode::Balanced, "thermal.mode_balanced"),
                (PerformanceMode::Performance, "thermal.mode_performance"),
            ] {
                let selected = cfg.thermal_fan.performance_mode == mode;
                if ui.selectable_label(selected, i18n.t(key)).clicked() {
                    cfg.thermal_fan.performance_mode = mode;
                    on_cmd(WorkerCommand::SetPerformanceMode(mode));
                }
            }
        });
    });

    ui.add_space(10.0);

    // 4 Metric Cards
    ui.columns(2, |cols| {
        card_frame(dark).show(&mut cols[0], |ui| {
            ui.label(RichText::new(i18n.t("dashboard.cpu")).strong().color(ACCENT_CYAN));
            let usage = st
                .telemetry
                .cpu_usage_pct
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into());
            let clock = st
                .telemetry
                .cpu_clock_mhz
                .map(|v| format!("{v} MHz"))
                .unwrap_or_else(|| "—".into());
            let temp = st
                .telemetry
                .cpu_temp_c
                .map(|v| format!("{v:.0} °C"))
                .unwrap_or_else(|| "N/A".into());
            ui.label(RichText::new(format!("{usage}  |  {clock}  |  {temp}")).size(18.0).strong());
            if let Some(diag) = &st.diagnostics {
                ui.weak(&diag.identity.cpu_name);
            }
        });

        card_frame(dark).show(&mut cols[1], |ui| {
            ui.label(RichText::new(i18n.t("dashboard.gpu")).strong().color(ACCENT_CYAN));
            let temp = st
                .telemetry
                .gpu_temp_c
                .map(|v| format!("{v:.0} °C"))
                .unwrap_or_else(|| "—".into());
            let load = st
                .telemetry
                .gpu_usage_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let pwr = st
                .telemetry
                .gpu_power_w
                .map(|v| format!("{v:.1} W"))
                .unwrap_or_else(|| "—".into());
            ui.label(RichText::new(format!("{load}  |  {temp}  |  {pwr}")).size(18.0).strong());
            ui.weak(
                st.telemetry
                    .gpu_name
                    .as_deref()
                    .unwrap_or("GPU Telemetry"),
            );
        });
    });

    ui.add_space(8.0);

    ui.columns(2, |cols| {
        card_frame(dark).show(&mut cols[0], |ui| {
            ui.label(RichText::new(i18n.t("dashboard.ram")).strong().color(ACCENT_CYAN));
            let used = st.telemetry.ram_used_gb.unwrap_or(0.0);
            let total = st.telemetry.ram_total_gb.unwrap_or(0.0);
            ui.label(RichText::new(format!("{used:.2} / {total:.2} GB")).size(18.0).strong());
            let ratio = if total > 0.0 {
                (used / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(ratio).show_percentage());
        });

        card_frame(dark).show(&mut cols[1], |ui| {
            ui.label(
                RichText::new(format!(
                    "{} & {}",
                    i18n.t("dashboard.fans"),
                    i18n.t("dashboard.battery")
                ))
                .strong()
                .color(ACCENT_CYAN),
            );
            let fan_txt = if st.telemetry.fan_rpms.is_empty() {
                "Fans: N/A (Admin)".to_string()
            } else {
                st.telemetry
                    .fan_rpms
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("F{}: {r} RPM", i + 1))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            let bat_pct = st
                .telemetry
                .battery_pct
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "—".into());
            let ac_txt = match st.telemetry.ac_connected {
                Some(true) => i18n.t("dashboard.power_source_ac"),
                Some(false) => i18n.t("dashboard.power_source_dc"),
                None => i18n.t("common.unknown"),
            };
            ui.label(RichText::new(format!("{bat_pct} ({ac_txt})")).size(16.0).strong());
            ui.weak(fan_txt);
        });
    });

    ui.add_space(10.0);

    // Real-time Temperature & Load Plot
    card_frame(dark).show(ui, |ui| {
        ui.label(RichText::new("Real-Time Temperature & Usage").strong());
        let gpu_temp_pts: PlotPoints<'_> = st
            .gpu_temp_history
            .iter()
            .map(|&(x, y)| [x, y as f64])
            .collect();
        let cpu_temp_pts: PlotPoints<'_> = st
            .cpu_temp_history
            .iter()
            .map(|&(x, y)| [x, y as f64])
            .collect();
        let cpu_load_pts: PlotPoints<'_> = st
            .cpu_usage_history
            .iter()
            .map(|&(x, y)| [x, y as f64])
            .collect();

        Plot::new("dashboard_telemetry_plot")
            .height(220.0)
            .legend(Legend::default())
            .include_y(0.0)
            .include_y(100.0)
            .allow_zoom(false)
            .allow_drag(false)
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new(gpu_temp_pts)
                        .name("GPU °C")
                        .color(Color32::from_rgb(0, 180, 216))
                        .width(2.0_f32),
                );
                if !st.cpu_temp_history.is_empty() {
                    plot_ui.line(
                        Line::new(cpu_temp_pts)
                            .name("CPU °C")
                            .color(Color32::from_rgb(245, 158, 11))
                            .width(2.0_f32),
                    );
                }
                plot_ui.line(
                    Line::new(cpu_load_pts)
                        .name("CPU %")
                        .color(Color32::from_rgb(34, 197, 94))
                        .width(1.5_f32),
                );
            });
    });
}
