use crate::theme::{card_frame, section_title, status_badge, STATUS_OK, STATUS_WARN};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Slider, Ui};
use omvix_core::{AppConfig, CpuBoostMode, I18n};

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();

    ui.heading(i18n.t("nav.tuning"));
    ui.add_space(6.0);

    // Security / VBS / HVCI Banner
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if st.security.hvci_running || st.security.vbs_status == Some(2) {
                status_badge(ui, "VBS / HVCI ACTIVE", STATUS_WARN);
            } else {
                status_badge(ui, "VBS / HVCI OFF", STATUS_OK);
            }
            if st.security.pawn_io_installed {
                status_badge(ui, "PawnIO READY", STATUS_OK);
            } else {
                status_badge(ui, "PawnIO NOT INSTALLED", STATUS_WARN);
            }
        });
        ui.add_space(4.0);
        if st.security.hvci_running || st.security.vbs_status == Some(2) {
            ui.colored_label(STATUS_WARN, i18n.t("tuning.vbs_hvci_warning"));
        } else {
            ui.label(&st.security.undervolt_note);
        }
    });

    ui.add_space(8.0);

    // CPU Turbo Boost Mode (Works without kernel driver!)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("tuning.cpu_boost"), t);
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
                if ui
                    .selectable_label(cur == mode, format!("{mode:?}"))
                    .clicked()
                {
                    cfg.tuning.cpu_boost_mode = Some(mode);
                    on_save();
                    on_cmd(WorkerCommand::SetCpuBoost(mode));
                }
            }
        });
    });

    ui.add_space(8.0);

    // Custom Power Limits (PL1 / PL2) — R4 / B1
    card_frame(t).show(ui, |ui| {
        section_title(
            ui,
            format!(
                "{} / {}",
                i18n.t("tuning.power_limit_pl1"),
                i18n.t("tuning.power_limit_pl2")
            ),
            t,
        );
        if !st.board_known {
            ui.colored_label(STATUS_WARN, i18n.t("tuning.power_limit_unknown_board"));
        }

        ui.add_enabled_ui(st.board_known, |ui| {
            let mut pl1_enabled = cfg.tuning.pl1_watts.is_some();
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut pl1_enabled, i18n.t("tuning.power_limit_pl1"))
                    .changed()
                {
                    cfg.tuning.pl1_watts = if pl1_enabled { Some(55) } else { None };
                    on_save();
                }
                if let Some(ref mut pl1) = cfg.tuning.pl1_watts {
                    ui.add(Slider::new(pl1, 15..=175).suffix(" W"));
                } else {
                    ui.weak(i18n.t("tuning.leave_unchanged"));
                }
            });

            let mut pl2_enabled = cfg.tuning.pl2_watts.is_some();
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut pl2_enabled, i18n.t("tuning.power_limit_pl2"))
                    .changed()
                {
                    cfg.tuning.pl2_watts = if pl2_enabled { Some(115) } else { None };
                    on_save();
                }
                if let Some(ref mut pl2) = cfg.tuning.pl2_watts {
                    ui.add(Slider::new(pl2, 20..=200).suffix(" W"));
                } else {
                    ui.weak(i18n.t("tuning.leave_unchanged"));
                }
            });

            let has_any_pl = cfg.tuning.pl1_watts.is_some() || cfg.tuning.pl2_watts.is_some();
            if ui
                .add_enabled(has_any_pl, egui::Button::new(i18n.t("common.apply")))
                .clicked()
            {
                on_save();
                on_cmd(WorkerCommand::SetPowerLimits {
                    pl1: cfg.tuning.pl1_watts,
                    pl2: cfg.tuning.pl2_watts,
                });
            }
        });
    });

    ui.add_space(8.0);

    // Custom Temperature Limit (TCC) & Core Undervolt — B3: disabled until PawnIO MSR write bridge is wired
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(
                ui,
                format!(
                    "{} / {}",
                    i18n.t("tuning.temp_limit"),
                    i18n.t("tuning.undervolt")
                ),
                t,
            );
            status_badge(ui, &i18n.t("tuning.msr_bridge_pending"), STATUS_WARN);
        });
        let tooltip = i18n.t("tuning.msr_bridge_unavailable_tooltip");
        let mut tcc_preview = cfg.tuning.tcc_temp_limit_c.unwrap_or(95);
        let mut uv_preview = cfg.tuning.core_undervolt_mv.unwrap_or(0);
        ui.add_enabled_ui(false, |ui| {
            ui.horizontal(|ui| {
                ui.label(i18n.t("tuning.temp_limit"))
                    .on_disabled_hover_text(&tooltip);
                ui.add(Slider::new(&mut tcc_preview, 70..=100).suffix(" °C"))
                    .on_disabled_hover_text(&tooltip);
                ui.button(i18n.t("common.apply"))
                    .on_disabled_hover_text(&tooltip);
            });
            ui.horizontal(|ui| {
                ui.label(i18n.t("tuning.undervolt"))
                    .on_disabled_hover_text(&tooltip);
                ui.add(Slider::new(&mut uv_preview, -150..=0).suffix(" mV"))
                    .on_disabled_hover_text(&tooltip);
                ui.button(i18n.t("common.apply"))
                    .on_disabled_hover_text(&tooltip);
            });
        });
        ui.weak(egui::RichText::new(tooltip).size(11.0));
    });
}
