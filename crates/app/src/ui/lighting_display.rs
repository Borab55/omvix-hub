use crate::theme::{
    accent_color, card_frame, pill_button, section_title, status_badge, STATUS_ERR, STATUS_WARN,
};
use crate::ui::control_committed;
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Slider, Ui};
use omvix_core::{AppConfig, GpuMode, I18n, KeyboardType, RgbColor};

#[derive(Debug, Default, Clone)]
pub struct LightingDisplayDraftState {
    pub pending_gpu_mode: Option<GpuMode>,
    pub reboot_dismissed: bool,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    draft: &mut LightingDisplayDraftState,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);

    ui.heading(i18n.t("nav.lighting_display"));
    ui.add_space(6.0);

    // Keyboard Backlight & RGB Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal(|ui| {
            section_title(ui, i18n.t("lighting.keyboard"), t);
            status_badge(ui, &format!("{:?}", st.keyboard_type), accent);
        });

        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut cfg.lighting.enabled, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyLighting);
            }
            ui.add_space(10.0);
            ui.label(i18n.t("lighting.brightness"));
            let resp = ui.add(Slider::new(&mut cfg.lighting.brightness_pct, 0..=100).suffix("%"));
            if control_committed(ui, "lighting_brightness", &resp) {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyLighting);
            }
        });

        let zone_count = match st.keyboard_type {
            KeyboardType::FourZoneRgb => 4,
            KeyboardType::WhiteBacklight => 0,
            _ => 1,
        };

        if zone_count > 0 {
            ui.add_space(4.0);
            let mut zone_committed = false;
            ui.horizontal_wrapped(|ui| {
                for (z, cur) in cfg.lighting.zones.iter_mut().take(zone_count).enumerate() {
                    let mut srgb = [cur.r, cur.g, cur.b];
                    ui.label(i18n.t_fmt("lighting.zone", &[("index", &(z + 1).to_string())]));
                    let resp = ui.color_edit_button_srgb(&mut srgb);
                    if resp.changed() {
                        *cur = RgbColor {
                            r: srgb[0],
                            g: srgb[1],
                            b: srgb[2],
                        };
                    }
                    if control_committed(ui, ("lighting_zone_color", z), &resp) {
                        zone_committed = true;
                    }
                }
            });
            if zone_committed {
                cfg.lighting.enabled = true;
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyLighting);
            }

            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("lighting.presets"));
                for (label, rgb) in [
                    (
                        "Cyan",
                        RgbColor {
                            r: 0,
                            g: 180,
                            b: 216,
                        },
                    ),
                    (
                        "White",
                        RgbColor {
                            r: 245,
                            g: 245,
                            b: 255,
                        },
                    ),
                    (
                        "Crimson",
                        RgbColor {
                            r: 220,
                            g: 38,
                            b: 38,
                        },
                    ),
                    (
                        "Emerald",
                        RgbColor {
                            r: 34,
                            g: 197,
                            b: 94,
                        },
                    ),
                ] {
                    if ui.button(label).clicked() {
                        cfg.lighting.zones = [rgb; 4];
                        cfg.lighting.enabled = true;
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                }
            });
        }
    });

    ui.add_space(8.0);

    // Display Refresh Rate Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("display.refresh_rate"), t);
        if let Some(disp) = &st.display_mode {
            ui.label(format!(
                "{}x{} @ {}",
                disp.width,
                disp.height,
                i18n.t_fmt("display.hz", &[("hz", &disp.current_hz.to_string())])
            ));
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
        } else {
            ui.weak("...");
        }
    });

    ui.add_space(8.0);

    // GPU MUX Switch Card (R6: 2-step confirmation + BitLocker/battery warning + reboot prompt)
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("gpu.mux_switch"), t);
        let cur = st.gpu_mode.unwrap_or(GpuMode::Unknown);
        ui.horizontal_wrapped(|ui| {
            for (mode, key) in [
                (GpuMode::Hybrid, "gpu.mode_hybrid"),
                (GpuMode::Discrete, "gpu.mode_discrete"),
                (GpuMode::Optimus, "gpu.mode_optimus"),
            ] {
                let selected = draft.pending_gpu_mode.unwrap_or(cur) == mode;
                if pill_button(ui, &i18n.t(key), selected, t).clicked() {
                    draft.pending_gpu_mode = Some(mode);
                }
            }
        });

        if let Some(pending) = draft.pending_gpu_mode {
            ui.add_space(6.0);
            ui.colored_label(
                STATUS_WARN,
                RichText::new(i18n.t_fmt(
                    "gpu.mux_confirm_prompt",
                    &[("mode", &format!("{pending:?}"))],
                ))
                .strong(),
            );
            ui.colored_label(
                STATUS_ERR,
                RichText::new(i18n.t("gpu.bitlocker_warning")).size(11.5),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if pill_button(ui, &i18n.t("gpu.confirm_apply"), true, t).clicked() {
                    cfg.tuning.preferred_gpu_mode = Some(pending);
                    draft.pending_gpu_mode = None;
                    draft.reboot_dismissed = false;
                    on_save();
                    on_cmd(WorkerCommand::SetGpuMode(pending));
                }
                if ui.button(i18n.t("common.cancel")).clicked() {
                    draft.pending_gpu_mode = None;
                }
            });
        } else {
            ui.add_space(4.0);
            ui.label(
                RichText::new(i18n.t("gpu.reboot_required")).color(Color32::from_rgb(245, 158, 11)),
            );
        }

        if st.gpu_mux_reboot_pending && !draft.reboot_dismissed {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(accent, RichText::new(i18n.t("gpu.reboot_prompt")).strong());
                if pill_button(ui, &i18n.t("gpu.reboot_now"), true, t).clicked() {
                    on_cmd(WorkerCommand::RebootSystem);
                }
                if ui.button(i18n.t("gpu.reboot_later")).clicked() {
                    draft.reboot_dismissed = true;
                }
            });
        }

        if !st.security.is_admin {
            ui.colored_label(STATUS_WARN, i18n.t("tuning.admin_required_warning"));
        }
    });
}
