use crate::theme::{
    STATUS_OK, STATUS_WARN, accent_color, card_frame, clickable_status_badge, section_title,
    status_badge,
};
use crate::ui::Tab;
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Slider, Ui};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryProfile, CpuBoostMode, FanMode, I18n, PerformanceMode,
    RgbColor,
};

#[derive(Default)]
pub struct HomeActions {
    pub navigate_to: Option<Tab>,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
    on_restart_admin: &mut dyn FnMut(),
) -> HomeActions {
    let mut actions = HomeActions::default();
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    ui.horizontal(|ui| {
        ui.heading(i18n.t("home.title"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(RichText::new(i18n.t("home.open_monitor")).color(accent).strong()).clicked() {
                actions.navigate_to = Some(Tab::Monitor);
            }
        });
    });
    ui.add_space(6.0);

    // Status row: instant Admin badge + power source + clickable conflicts link
    if !st.security.is_admin {
        card_frame(t).show(ui, |ui| {
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
        ui.add_space(6.0);
    } else {
        ui.horizontal_wrapped(|ui| {
            status_badge(ui, &i18n.t("app.admin_active"), STATUS_OK);

            let ac_txt = match st.telemetry.ac_connected {
                Some(true) => i18n.t("dashboard.power_source_ac"),
                Some(false) => i18n.t("dashboard.power_source_dc"),
                None => i18n.t("common.unknown"),
            };
            status_badge(ui, &ac_txt, accent);

            let running_conflicts = st
                .conflict_services
                .iter()
                .filter(|s| s.status.eq_ignore_ascii_case("Running"))
                .count();
            if running_conflicts > 0 {
                let msg = i18n.t_fmt(
                    "conflicts.detected_summary",
                    &[("count", &running_conflicts.to_string())],
                );
                if clickable_status_badge(ui, &msg, STATUS_WARN).clicked() {
                    actions.navigate_to = Some(Tab::Diagnostics);
                }
            }
        });
        ui.add_space(6.0);
    }

    // Compact 2x2 Live Telemetry Summary Strip
    ui.columns(2, |cols| {
        card_frame(t).show(&mut cols[0], |ui| {
            section_title(ui, i18n.t("dashboard.cpu"), t);
            let usage = st
                .telemetry
                .cpu_usage_pct
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into());
            let temp = st
                .telemetry
                .cpu_temp_c
                .map(|v| format!("{v:.0} °C"))
                .unwrap_or_else(|| "—".into());
            let pwr = st
                .telemetry
                .cpu_power_w
                .map(|v| format!("{v:.1} W"))
                .unwrap_or_else(|| "—".into());
            ui.label(RichText::new(format!("{usage}  |  {temp}  |  {pwr}")).size(17.5).strong());
            let clk = st
                .telemetry
                .cpu_clock_mhz
                .map(|v| format!("{v} MHz"))
                .unwrap_or_else(|| "—".into());
            let cpu_name = st
                .diagnostics
                .as_ref()
                .map(|d| d.identity.cpu_name.as_str())
                .unwrap_or("CPU");
            ui.weak(format!("{clk}  •  {cpu_name}"));
        });

        card_frame(t).show(&mut cols[1], |ui| {
            section_title(ui, i18n.t("dashboard.gpu"), t);
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
            ui.label(RichText::new(format!("{load}  |  {temp}  |  {pwr}")).size(17.5).strong());
            let vram = match (st.telemetry.gpu_mem_used_mb, st.telemetry.gpu_mem_total_mb) {
                (Some(u), Some(tot)) => format!("{:.1}/{:.1} GB VRAM", u as f32 / 1024.0, tot as f32 / 1024.0),
                _ => st.telemetry.gpu_name.clone().unwrap_or_else(|| "GPU".into()),
            };
            ui.weak(vram);
        });
    });

    ui.add_space(6.0);

    ui.columns(2, |cols| {
        card_frame(t).show(&mut cols[0], |ui| {
            section_title(ui, i18n.t("dashboard.ram"), t);
            let used = st.telemetry.ram_used_gb.unwrap_or(0.0);
            let total = st.telemetry.ram_total_gb.unwrap_or(0.0);
            ui.label(RichText::new(format!("{used:.2} / {total:.2} GB")).size(17.0).strong());
            let ratio = if total > 0.0 {
                (used / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(ratio).show_percentage());
        });

        card_frame(t).show(&mut cols[1], |ui| {
            section_title(
                ui,
                format!("{} & {}", i18n.t("dashboard.fans"), i18n.t("dashboard.battery")),
                t,
            );
            let bat_pct = st
                .telemetry
                .battery_pct
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "—".into());
            let rate = st
                .telemetry
                .battery_rate_w
                .map(|w| format!("{w:+.1} W"))
                .unwrap_or_else(|| "—".into());
            let fan_txt = if st.telemetry.fan_rpms.is_empty() {
                "Fans: — (Admin)".to_string()
            } else {
                st.telemetry
                    .fan_rpms
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("F{}: {r} RPM", i + 1))
                    .collect::<Vec<_>>()
                    .join("  •  ")
            };
            ui.label(RichText::new(format!("{bat_pct}  ({rate})")).size(17.0).strong());
            ui.weak(fan_txt);
        });
    });

    ui.add_space(10.0);

    // Quick Controls Cockpit: Row 1 (Thermal Mode & Fan Mode)
    ui.columns(2, |cols| {
        card_frame(t).show(&mut cols[0], |ui| {
            section_title(ui, i18n.t("thermal.mode"), t);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (mode, key) in [
                    (PerformanceMode::Eco, "thermal.mode_eco"),
                    (PerformanceMode::Quiet, "thermal.mode_quiet"),
                    (PerformanceMode::Balanced, "thermal.mode_balanced"),
                    (PerformanceMode::Performance, "thermal.mode_performance"),
                ] {
                    let selected = cfg.thermal_fan.performance_mode == mode;
                    if ui.selectable_label(selected, i18n.t(key)).clicked() {
                        cfg.thermal_fan.performance_mode = mode;
                        on_save();
                        on_cmd(WorkerCommand::SetPerformanceMode(mode));
                    }
                }
            });
        });

        card_frame(t).show(&mut cols[1], |ui| {
            section_title(ui, i18n.t("fan.mode"), t);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (mode, key) in [
                    (FanMode::Auto, "fan.mode_auto"),
                    (FanMode::Max, "fan.mode_max"),
                    (FanMode::Manual, "fan.mode_manual"),
                    (FanMode::CustomCurve, "fan.mode_curve"),
                ] {
                    let selected = cfg.thermal_fan.fan_mode == mode;
                    if ui.selectable_label(selected, i18n.t(key)).clicked() {
                        cfg.thermal_fan.fan_mode = mode;
                        on_save();
                        on_cmd(WorkerCommand::SetFanMode {
                            mode,
                            manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                        });
                    }
                }
            });
            if cfg.thermal_fan.fan_mode == FanMode::Manual {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(Slider::new(&mut cfg.thermal_fan.manual_speed_pct, 0..=100).suffix("%"));
                    if ui.button(i18n.t("common.apply")).clicked() {
                        on_save();
                        on_cmd(WorkerCommand::SetFanMode {
                            mode: FanMode::Manual,
                            manual_speed_pct: Some(cfg.thermal_fan.manual_speed_pct),
                        });
                    }
                });
            }
        });
    });

    ui.add_space(8.0);

    // Quick Controls Cockpit: Row 2 (Battery Charge Limit Presets & Display Hz)
    ui.columns(2, |cols| {
        card_frame(t).show(&mut cols[0], |ui| {
            ui.horizontal(|ui| {
                section_title(ui, i18n.t("home.quick_battery"), t);
                ui.add_space(6.0);
                if ui
                    .checkbox(&mut cfg.battery.enabled, i18n.t("common.enabled"))
                    .changed()
                {
                    if !cfg.battery.enabled {
                        cfg.battery.mode = BatteryChargeMode::Auto;
                    } else if cfg.battery.mode == BatteryChargeMode::Auto {
                        cfg.battery.mode = BatteryChargeMode::Inhibit;
                    }
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyBatteryNow);
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for preset in BatteryProfile::builtins() {
                    let label = match preset.id.as_str() {
                        "desktop" => i18n.t("battery.preset_desktop"),
                        "balanced" => i18n.t("battery.preset_balanced"),
                        "travel" => i18n.t("battery.preset_travel"),
                        "full" => i18n.t("battery.preset_full"),
                        _ => preset.name.clone(),
                    };
                    let selected = is_profile_active(cfg, &preset);
                    if ui.selectable_label(selected, label).clicked() {
                        apply_battery_profile(cfg, &preset);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                }
                for custom in cfg.battery.custom_profiles.clone() {
                    let selected = cfg.battery.active_profile_id.as_deref() == Some(&custom.id);
                    if ui
                        .selectable_label(
                            selected,
                            format!("{} ({}%)", custom.name, custom.stop_threshold_pct),
                        )
                        .clicked()
                    {
                        apply_battery_profile(cfg, &custom);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                }
            });
            ui.weak(format!(
                "{}% / {}%  •  {:?}",
                cfg.battery.stop_threshold_pct, cfg.battery.start_threshold_pct, cfg.battery.mode
            ));
        });

        card_frame(t).show(&mut cols[1], |ui| {
            section_title(ui, i18n.t("home.quick_display"), t);
            ui.add_space(4.0);
            if let Some(disp) = &st.display_mode {
                ui.horizontal_wrapped(|ui| {
                    for &hz in &disp.supported_hz {
                        let selected = disp.current_hz == hz;
                        if ui
                            .selectable_label(
                                selected,
                                i18n.t_fmt("display.hz", &[("hz", &hz.to_string())]),
                            )
                            .clicked()
                        {
                            on_cmd(WorkerCommand::SetRefreshRate(hz));
                        }
                    }
                });
                ui.weak(format!("{}x{} @ {} Hz", disp.width, disp.height, disp.current_hz));
            } else {
                ui.weak("...");
            }
        });
    });

    ui.add_space(8.0);

    // Quick Controls Cockpit: Row 3 (Keyboard Lighting & CPU Turbo Boost)
    ui.columns(2, |cols| {
        card_frame(t).show(&mut cols[0], |ui| {
            ui.horizontal(|ui| {
                section_title(ui, i18n.t("home.quick_keyboard"), t);
                if ui
                    .checkbox(&mut cfg.lighting.enabled, i18n.t("common.enabled"))
                    .changed()
                {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyLighting);
                }
            });
            ui.horizontal(|ui| {
                ui.label(i18n.t("lighting.brightness"));
                ui.add(Slider::new(&mut cfg.lighting.brightness_pct, 0..=100).suffix("%"));
                if ui.button(i18n.t("common.apply")).clicked() {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyLighting);
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (name, rgb) in [
                    ("Cyan", RgbColor { r: 0, g: 180, b: 216 }),
                    ("White", RgbColor { r: 245, g: 245, b: 255 }),
                    ("Red", RgbColor { r: 220, g: 38, b: 38 }),
                    ("Green", RgbColor { r: 34, g: 197, b: 94 }),
                ] {
                    let swatch = Color32::from_rgb(rgb.r, rgb.g, rgb.b);
                    if ui.button(RichText::new(format!("■ {name}")).color(swatch)).clicked() {
                        cfg.lighting.zones = [rgb; 4];
                        cfg.lighting.enabled = true;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                }
            });
        });

        card_frame(t).show(&mut cols[1], |ui| {
            section_title(ui, i18n.t("home.quick_cpu_boost"), t);
            ui.add_space(4.0);
            let cur = st
                .cpu_boost
                .or(cfg.tuning.cpu_boost_mode)
                .unwrap_or(CpuBoostMode::Aggressive);
            ui.horizontal_wrapped(|ui| {
                for mode in [
                    CpuBoostMode::Disabled,
                    CpuBoostMode::Enabled,
                    CpuBoostMode::Aggressive,
                    CpuBoostMode::EfficientAggressive,
                ] {
                    if ui.selectable_label(cur == mode, format!("{mode:?}")).clicked() {
                        cfg.tuning.cpu_boost_mode = Some(mode);
                        on_save();
                        on_cmd(WorkerCommand::SetCpuBoost(mode));
                    }
                }
            });
        });
    });

    actions
}

pub fn apply_battery_profile(cfg: &mut AppConfig, profile: &BatteryProfile) {
    cfg.battery.enabled = profile.enabled;
    cfg.battery.stop_threshold_pct = profile.stop_threshold_pct;
    cfg.battery.start_threshold_pct = profile.start_threshold_pct;
    cfg.battery.mode = profile.mode;
    cfg.battery.active_profile_id = Some(profile.id.clone());
}

pub fn is_profile_active(cfg: &AppConfig, profile: &BatteryProfile) -> bool {
    if cfg.battery.active_profile_id.as_deref() == Some(&profile.id) {
        return true;
    }
    cfg.battery.enabled == profile.enabled
        && cfg.battery.stop_threshold_pct == profile.stop_threshold_pct
        && cfg.battery.start_threshold_pct == profile.start_threshold_pct
        && cfg.battery.mode == profile.mode
}
