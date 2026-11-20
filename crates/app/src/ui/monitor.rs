use crate::theme::{
    STATUS_ERR, STATUS_OK, STATUS_WARN, accent_color, card_frame, pill_button, section_title,
    status_badge,
};
use crate::ui::charts::{ChartSeries, telemetry_area_chart};
use crate::worker::{MetricStats, SharedState, WorkerCommand};
use egui::{Color32, RichText, Ui};
use omvix_core::{AppConfig, I18n};

const REFRESH_OPTIONS_MS: &[(u64, &str)] = &[
    (100, "0.1s"),
    (250, "0.25s"),
    (500, "0.5s"),
    (1000, "1s"),
    (1500, "1.5s"),
    (2000, "2s"),
    (5000, "5s"),
];

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    ui.heading(i18n.t("monitor.title"));
    ui.add_space(6.0);

    if st.thermal_guard_active {
        card_frame(t).show(ui, |ui| {
            status_badge(ui, &i18n.t("monitor.thermal_guard_active"), STATUS_ERR);
        });
        ui.add_space(6.0);
    }

    // Refresh Rate + Pause/Resume + Reset Stats bar
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("monitor.refresh_rate"), t);
            ui.add_space(6.0);
            for &(ms, label) in REFRESH_OPTIONS_MS {
                let selected = cfg.general.telemetry_interval_ms == ms;
                if ui.selectable_label(selected, label).clicked() {
                    cfg.general.telemetry_interval_ms = ms;
                    on_save();
                    on_cmd(WorkerCommand::SetTelemetryIntervalMs(ms));
                }
            }

            ui.separator();
            let pause_label = if st.telemetry_paused {
                i18n.t("monitor.resume")
            } else {
                i18n.t("monitor.pause")
            };
            if ui
                .selectable_label(st.telemetry_paused, pause_label)
                .clicked()
            {
                on_cmd(WorkerCommand::SetTelemetryPaused(!st.telemetry_paused));
            }
            if ui.button(i18n.t("monitor.reset_stats")).clicked() {
                on_cmd(WorkerCommand::ResetTelemetryStats);
            }
        });
        ui.weak(i18n.t("monitor.ec_note"));
    });

    ui.add_space(8.0);

    // Detailed 4 Telemetry Cards (2x2 Grid)
    ui.columns(2, |cols| {
        // Card 1: CPU
        card_frame(t).show(&mut cols[0], |ui| {
            ui.horizontal(|ui| {
                section_title(ui, i18n.t("dashboard.cpu"), t);
                if st.pawnio_cpu_active {
                    status_badge(ui, "PawnIO", STATUS_OK);
                }
            });
            let usage = st
                .telemetry
                .cpu_usage_pct
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into());
            let temp = match (st.telemetry.cpu_temp_c, st.telemetry.chassis_temp_c) {
                (Some(c), _) => format!("{c:.0} °C"),
                (None, Some(ch)) => format!("{}: {ch:.0} °C", i18n.t("cockpit.chassis_short")),
                (None, None) => "—".into(),
            };
            let clk = st
                .telemetry
                .cpu_clock_mhz
                .map(|v| format!("{v} MHz"))
                .unwrap_or_else(|| "—".into());
            let pwr = st
                .telemetry
                .cpu_power_w
                .map(|v| format!("{v:.1} W"))
                .unwrap_or_else(|| "—".into());
            ui.label(
                RichText::new(format!("{usage}  |  {temp}  |  {pwr}  |  {clk}"))
                    .size(16.5)
                    .strong(),
            );
            ui.weak(format!(
                "{}: °C [{}]  •  W [{}]",
                i18n.t("monitor.session_minmax"),
                fmt_stats(&st.cpu_temp_stats, "°C"),
                fmt_stats(&st.cpu_power_stats, "W"),
            ));
            if st.telemetry.cpu_temp_c.is_none() {
                ui.add_space(4.0);
                ui.weak(RichText::new(i18n.t("monitor.cpu_temp_pawnio_hint")).size(11.0));
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    if st.pawnio_installing {
                        ui.weak(i18n.t("monitor.installing_pawnio"));
                    } else if pill_button(ui, &i18n.t("monitor.install_pawnio"), true, t).clicked() {
                        on_cmd(WorkerCommand::InstallPawnIo);
                    }
                });
            }
        });

        // Card 2: GPU
        card_frame(t).show(&mut cols[1], |ui| {
            ui.horizontal(|ui| {
                section_title(ui, i18n.t("dashboard.gpu"), t);
                if let Some(pstate) = &st.telemetry.gpu_pstate {
                    status_badge(ui, pstate, accent);
                }
                match &st.telemetry.gpu_throttle_reason {
                    Some(reason) => status_badge(ui, reason, STATUS_WARN),
                    None if st.telemetry.gpu_name.is_some() => {
                        status_badge(ui, &i18n.t("monitor.no_throttle"), STATUS_OK);
                    }
                    _ => {}
                }
            });

            let load = st
                .telemetry
                .gpu_usage_pct
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "—".into());
            let temp = st
                .telemetry
                .gpu_temp_c
                .map(|v| format!("{v:.0} °C"))
                .unwrap_or_else(|| "—".into());
            let pwr = st
                .telemetry
                .gpu_power_w
                .map(|v| format!("{v:.1} W"))
                .unwrap_or_else(|| "—".into());
            let limit = st
                .telemetry
                .gpu_power_limit_w
                .map(|v| format!("/ {v:.0} W"))
                .unwrap_or_default();
            ui.label(
                RichText::new(format!("{load}  |  {temp}  |  {pwr} {limit}"))
                    .size(16.5)
                    .strong(),
            );

            let vram = match (st.telemetry.gpu_mem_used_mb, st.telemetry.gpu_mem_total_mb) {
                (Some(u), Some(tot)) => format!("{u} / {tot} MB"),
                _ => "—".into(),
            };
            let clocks = match (st.telemetry.gpu_clock_mhz, st.telemetry.gpu_mem_clock_mhz) {
                (Some(c), Some(m)) => format!("{c} / {m} MHz"),
                _ => "—".into(),
            };
            ui.weak(format!(
                "VRAM: {vram}  •  Clocks: {clocks}  •  °C [{}]",
                fmt_stats(&st.gpu_temp_stats, "°C")
            ));
        });
    });

    ui.add_space(6.0);

    ui.columns(2, |cols| {
        // Card 3: Fans & Battery
        card_frame(t).show(&mut cols[0], |ui| {
            section_title(
                ui,
                format!("{} & {}", i18n.t("dashboard.fans"), i18n.t("dashboard.battery")),
                t,
            );
            let fans_str = if st.telemetry.fan_rpms.is_empty() {
                "Fans: N/A (Admin)".to_string()
            } else {
                st.telemetry
                    .fan_rpms
                    .iter()
                    .enumerate()
                    .map(|(i, rpm)| {
                        let pct = st
                            .telemetry
                            .fan_max_rpms
                            .get(i)
                            .copied()
                            .filter(|&m| m > 0)
                            .map(|max_r| format!(" ({}%)", ((*rpm as f32 / max_r as f32) * 100.0).round().clamp(0.0, 100.0) as u32))
                            .unwrap_or_default();
                        format!("F{}: {rpm} RPM{pct}", i + 1)
                    })
                    .collect::<Vec<_>>()
                    .join("   ")
            };
            ui.label(RichText::new(fans_str).size(15.5).strong());

            let bat = st
                .telemetry
                .battery_pct
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "—".into());
            let rate = st
                .telemetry
                .battery_rate_w
                .map(|w| format!("{w:+.1} W"))
                .unwrap_or_else(|| "—".into());
            let volt = st
                .telemetry
                .battery_voltage_v
                .map(|v| format!("{v:.2} V"))
                .unwrap_or_else(|| "—".into());
            let eta = st
                .telemetry
                .battery_time_remaining_min
                .map(|m| format!("  •  {}h {}m", m / 60, m % 60))
                .unwrap_or_default();
            ui.weak(format!("Bat: {bat}  |  {rate}  |  {volt}{eta}"));
        });

        // Card 4: RAM, Disk & Network
        card_frame(t).show(&mut cols[1], |ui| {
            section_title(ui, i18n.t("monitor.storage_network"), t);
            let used = st.telemetry.ram_used_gb.unwrap_or(0.0);
            let total = st.telemetry.ram_total_gb.unwrap_or(0.0);
            let ratio = if total > 0.0 {
                (used / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.label(RichText::new(format!("RAM: {used:.2} / {total:.2} GB")).size(15.5).strong());
            ui.add(egui::ProgressBar::new(ratio).show_percentage());

            let dr = st
                .telemetry
                .disk_read_mb_s
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "—".into());
            let dw = st
                .telemetry
                .disk_write_mb_s
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "—".into());
            let rx = st
                .telemetry
                .net_rx_mb_s
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into());
            let tx = st
                .telemetry
                .net_tx_mb_s
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into());
            ui.weak(format!(
                "Disk R/W: {dr} / {dw} MB/s   •   Net ↓/↑: {rx} / {tx} MB/s"
            ));
        });
    });

    ui.add_space(8.0);

    // Graph 1: Temperature (°C)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("monitor.graph_temperature"), t);
        let cpu_pts: Vec<(f64, f32)> = st.cpu_temp_history.iter().copied().collect();
        let chassis_pts: Vec<(f64, f32)> = st.chassis_temp_history.iter().copied().collect();
        let gpu_pts: Vec<(f64, f32)> = st.gpu_temp_history.iter().copied().collect();

        telemetry_area_chart(
            ui,
            "monitor_temp_chart",
            175.0,
            20.0,
            105.0,
            "°C",
            &[
                ChartSeries {
                    name: "CPU",
                    color: Color32::from_rgb(245, 158, 11),
                    unit: "°C",
                    points: &cpu_pts,
                    fill_alpha: 34,
                },
                ChartSeries {
                    name: "GPU",
                    color: accent,
                    unit: "°C",
                    points: &gpu_pts,
                    fill_alpha: 30,
                },
                ChartSeries {
                    name: &i18n.t("cockpit.chassis_short"),
                    color: Color32::from_rgb(160, 165, 175),
                    unit: "°C",
                    points: &chassis_pts,
                    fill_alpha: 20,
                },
            ],
            t,
        );
    });

    ui.add_space(8.0);

    // Graph 2: Utilization (%)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("monitor.graph_usage"), t);
        let cpu_pts: Vec<(f64, f32)> = st.cpu_usage_history.iter().copied().collect();
        let gpu_pts: Vec<(f64, f32)> = st.gpu_load_history.iter().copied().collect();
        let ram_pts: Vec<(f64, f32)> = st.ram_usage_history.iter().copied().collect();

        telemetry_area_chart(
            ui,
            "monitor_usage_chart",
            175.0,
            0.0,
            100.0,
            "%",
            &[
                ChartSeries {
                    name: "CPU",
                    color: Color32::from_rgb(34, 197, 94),
                    unit: "%",
                    points: &cpu_pts,
                    fill_alpha: 32,
                },
                ChartSeries {
                    name: "GPU",
                    color: accent,
                    unit: "%",
                    points: &gpu_pts,
                    fill_alpha: 28,
                },
                ChartSeries {
                    name: "RAM",
                    color: Color32::from_rgb(168, 85, 247),
                    unit: "%",
                    points: &ram_pts,
                    fill_alpha: 16,
                },
            ],
            t,
        );
    });

    ui.add_space(8.0);

    // Graph 3: Power (W)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("monitor.graph_power"), t);
        let cpu_pwr: Vec<(f64, f32)> = st.cpu_power_history.iter().copied().collect();
        let gpu_pwr: Vec<(f64, f32)> = st.gpu_power_history.iter().copied().collect();
        let bat_pwr: Vec<(f64, f32)> = st.battery_power_history.iter().copied().collect();

        telemetry_area_chart(
            ui,
            "monitor_power_chart",
            175.0,
            0.0,
            60.0,
            "W",
            &[
                ChartSeries {
                    name: "CPU",
                    color: Color32::from_rgb(249, 115, 22),
                    unit: "W",
                    points: &cpu_pwr,
                    fill_alpha: 34,
                },
                ChartSeries {
                    name: "GPU",
                    color: accent,
                    unit: "W",
                    points: &gpu_pwr,
                    fill_alpha: 28,
                },
                ChartSeries {
                    name: "Bat",
                    color: Color32::from_rgb(34, 197, 94),
                    unit: "W",
                    points: &bat_pwr,
                    fill_alpha: 18,
                },
            ],
            t,
        );
    });
}

fn fmt_stats(stats: &MetricStats, unit: &str) -> String {
    match (stats.min_opt(), stats.max_opt(), stats.avg_opt()) {
        (Some(mn), Some(mx), Some(av)) => format!("{mn:.0} / {mx:.0} / {av:.0} {unit}"),
        _ => "—".to_string(),
    }
}
