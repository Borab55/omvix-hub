use crate::theme::{ACCENT_CYAN, STATUS_OK, STATUS_WARN, card_frame, status_badge};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, BatteryChargeMode, I18n};

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    dark: bool,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    ui.heading(i18n.t("nav.battery"));
    ui.add_space(6.0);

    if st.hp_charge_service_running {
        card_frame(dark).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(ui, "hp-charge-limit-rs", STATUS_WARN);
                ui.label(i18n.t("battery.conflict_service_warning"));
            });
        });
        ui.add_space(8.0);
    }

    // Live Battery Status Card
    card_frame(dark).show(ui, |ui| {
        if let Some(b) = &st.battery_status {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}%", b.charge_percent))
                        .size(26.0)
                        .strong()
                        .color(ACCENT_CYAN),
                );
                ui.add_space(10.0);
                let src = if b.ac_connected {
                    i18n.t("dashboard.power_source_ac")
                } else {
                    i18n.t("dashboard.power_source_dc")
                };
                status_badge(
                    ui,
                    &format!("{src} | {:?}", b.current_mode),
                    if b.charging { STATUS_WARN } else { STATUS_OK },
                );
            });

            ui.add(egui::ProgressBar::new(b.charge_percent as f32 / 100.0).show_percentage());
            ui.add_space(6.0);

            egui::Grid::new("battery_metrics_grid")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    ui.label(i18n.t("battery.wattage"));
                    ui.label(
                        b.charge_rate_w
                            .map(|w| format!("{w:+.2} W"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();

                    ui.label(i18n.t("battery.health"));
                    let cap_str = match (b.full_charge_capacity_mwh, b.design_capacity_mwh) {
                        (Some(f), Some(d)) => format!("{f} / {d} mWh"),
                        _ => "N/A".into(),
                    };
                    ui.label(cap_str);
                    ui.end_row();

                    ui.label(i18n.t("battery.wear_level"));
                    ui.label(
                        b.wear_percent
                            .map(|w| format!("{w:.1}%"))
                            .unwrap_or_else(|| "N/A".into()),
                    );
                    ui.end_row();
                });
        } else {
            ui.weak("Waiting for battery telemetry...");
        }
    });

    ui.add_space(8.0);

    // Charge Limiting Controls Card
    card_frame(dark).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(i18n.t("battery.charge_limit")).strong().color(ACCENT_CYAN));
            if ui
                .checkbox(&mut cfg.battery.enabled, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(i18n.t("battery.hold_mode"));
            for (m, key) in [
                (BatteryChargeMode::Inhibit, "battery.mode_inhibit"),
                (BatteryChargeMode::Hold, "battery.mode_hold"),
                (BatteryChargeMode::Auto, "battery.mode_auto"),
            ] {
                if ui
                    .selectable_label(cfg.battery.mode == m, i18n.t(key))
                    .clicked()
                {
                    cfg.battery.mode = m;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(i18n.t("battery.stop_threshold"));
            ui.add(Slider::new(&mut cfg.battery.stop_threshold_pct, 50..=100).suffix("%"));
        });

        let max_start = cfg.battery.stop_threshold_pct.saturating_sub(2).max(40);
        if cfg.battery.start_threshold_pct > max_start {
            cfg.battery.start_threshold_pct = max_start;
        }
        ui.horizontal(|ui| {
            ui.label(i18n.t("battery.start_threshold"));
            ui.add(Slider::new(&mut cfg.battery.start_threshold_pct, 40..=max_start).suffix("%"));
        });

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(RichText::new(i18n.t("common.apply")).strong())
                .clicked()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyBatteryNow);
            }

            let travel_btn_label = if cfg.battery.one_time_full_charge {
                format!("✓ {}", i18n.t("battery.one_time_full_charge"))
            } else {
                i18n.t("battery.one_time_full_charge")
            };
            if ui.selectable_label(cfg.battery.one_time_full_charge, travel_btn_label).clicked() {
                cfg.battery.one_time_full_charge = !cfg.battery.one_time_full_charge;
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyBatteryNow);
            }
        });

        if cfg.battery.one_time_full_charge {
            ui.add_space(4.0);
            ui.colored_label(STATUS_WARN, i18n.t("battery.one_time_active"));
        } else {
            ui.weak(i18n.t("battery.travel_mode_desc"));
        }
    });
}
