use crate::theme::{STATUS_OK, STATUS_WARN, card_frame, section_title, status_badge};
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
                if ui.selectable_label(cur == mode, format!("{mode:?}")).clicked() {
                    cfg.tuning.cpu_boost_mode = Some(mode);
                    on_save();
                    on_cmd(WorkerCommand::SetCpuBoost(mode));
                }
            }
        });
    });

    ui.add_space(8.0);

    // Custom Power Limits (PL1 / PL2)
    card_frame(t).show(ui, |ui| {
        section_title(ui, format!("{} / {}", i18n.t("tuning.power_limit_pl1"), i18n.t("tuning.power_limit_pl2")), t);
        let pl1 = cfg.tuning.pl1_watts.get_or_insert(55);
        ui.horizontal(|ui| {
            ui.label(i18n.t("tuning.power_limit_pl1"));
            ui.add(Slider::new(pl1, 15..=175).suffix(" W"));
        });
        let pl2 = cfg.tuning.pl2_watts.get_or_insert(115);
        ui.horizontal(|ui| {
            ui.label(i18n.t("tuning.power_limit_pl2"));
            ui.add(Slider::new(pl2, 20..=200).suffix(" W"));
        });
        if ui.button(i18n.t("common.apply")).clicked() {
            on_save();
            on_cmd(WorkerCommand::SetPowerLimits {
                pl1: cfg.tuning.pl1_watts,
                pl2: cfg.tuning.pl2_watts,
            });
        }
    });

    ui.add_space(8.0);

    // Custom Temperature Limit (TCC) & Core Undervolt
    card_frame(t).show(ui, |ui| {
        section_title(ui, format!("{} / {}", i18n.t("tuning.temp_limit"), i18n.t("tuning.undervolt")), t);
        let tcc = cfg.tuning.tcc_temp_limit_c.get_or_insert(95);
        ui.horizontal(|ui| {
            ui.label(i18n.t("tuning.temp_limit"));
            ui.add(Slider::new(tcc, 70..=100).suffix(" °C"));
            if ui.button(i18n.t("common.apply")).clicked() {
                on_save();
                on_cmd(WorkerCommand::SetTccLimit(*tcc));
            }
        });

        let uv = cfg.tuning.core_undervolt_mv.get_or_insert(0);
        ui.horizontal(|ui| {
            ui.label(i18n.t("tuning.undervolt"));
            ui.add(Slider::new(uv, -150..=0).suffix(" mV"));
            if ui.button(i18n.t("common.apply")).clicked() {
                on_save();
                on_cmd(WorkerCommand::SetUndervolt(*uv));
            }
        });
    });
}
