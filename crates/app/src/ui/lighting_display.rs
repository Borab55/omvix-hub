use crate::theme::{
    accent_color, card_frame, fixed_color_picker_button, lighting_swatch_button, pill_button,
    section_title, section_toggle_button, status_badge, status_err, status_warn, ColorButtonStyle,
};
use crate::ui::{control_committed, unique_profile_slug};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, GpuMode, I18n, KeyboardType, LightingProfile, RgbColor};

#[derive(Debug, Default, Clone)]
pub struct LightingDisplayDraftState {
    pub pending_gpu_mode: Option<GpuMode>,
    pub reboot_dismissed: bool,
    pub profile_name: String,
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
    let warn_c = status_warn(t);
    let err_c = status_err(t);

    ui.heading(i18n.t("nav.lighting_display"));
    ui.add_space(6.0);

    // Keyboard Backlight & RGB Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if section_toggle_button(
                ui,
                &i18n.t("lighting.keyboard"),
                &mut cfg.lighting.enabled,
                &i18n.t("common.enabled"),
                t,
            )
            .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                on_cmd(WorkerCommand::ApplyLighting);
            }
            status_badge(ui, &format!("{:?}", st.keyboard_type), accent);
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("lighting.brightness"));
            let resp = ui.add(Slider::new(&mut cfg.lighting.brightness_pct, 0..=100).suffix("%"));
            if resp.changed() {
                cfg.lighting.active_profile_id = None;
            }
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
                    let resp = fixed_color_picker_button(
                        ui,
                        ("lighting_zone_picker", z),
                        &mut srgb,
                        ColorButtonStyle::ColorWheel { size: 22.0 },
                        t,
                    );
                    if resp.changed() {
                        *cur = RgbColor {
                            r: srgb[0],
                            g: srgb[1],
                            b: srgb[2],
                        };
                        cfg.lighting.enabled = true;
                        cfg.lighting.active_profile_id = None;
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

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("lighting.presets"));
                for preset in LightingProfile::builtins() {
                    let selected = is_lighting_profile_active(cfg, &preset);
                    if lighting_swatch_button(ui, &preset.zones, selected, &preset.name, t)
                        .clicked()
                    {
                        apply_lighting_profile(cfg, &preset);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                    if pill_button(ui, &preset.name, selected, t).clicked() {
                        apply_lighting_profile(cfg, &preset);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                }
            });

            ui.separator();
            section_title(ui, i18n.t("lighting.profiles_title"), t);
            ui.add_space(4.0);

            if cfg.lighting.custom_profiles.is_empty() {
                ui.weak(i18n.t("lighting.no_custom_profiles"));
            } else {
                let mut apply_prof: Option<LightingProfile> = None;
                let mut delete_idx: Option<usize> = None;
                for (idx, prof) in cfg.lighting.custom_profiles.iter().enumerate() {
                    let is_active = is_lighting_profile_active(cfg, prof);
                    ui.horizontal_wrapped(|ui| {
                        if lighting_swatch_button(ui, &prof.zones, is_active, &prof.name, t)
                            .clicked()
                        {
                            apply_prof = Some(prof.clone());
                        }
                        if pill_button(
                            ui,
                            &format!("{} ({}%)", prof.name, prof.brightness_pct),
                            is_active,
                            t,
                        )
                        .clicked()
                        {
                            apply_prof = Some(prof.clone());
                        }
                        if ui.button(i18n.t("lighting.delete_profile")).clicked() {
                            delete_idx = Some(idx);
                        }
                    });
                }
                if let Some(p) = apply_prof {
                    apply_lighting_profile(cfg, &p);
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyLighting);
                }
                if let Some(idx) = delete_idx {
                    let removed = cfg.lighting.custom_profiles.remove(idx);
                    if cfg.lighting.active_profile_id.as_deref() == Some(&removed.id) {
                        cfg.lighting.active_profile_id = None;
                    }
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            }

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("lighting.profile_name"));
                ui.add(
                    egui::TextEdit::singleline(&mut draft.profile_name)
                        .desired_width(175.0)
                        .clip_text(true),
                );
                if ui
                    .button(RichText::new(i18n.t("lighting.save_profile")).strong())
                    .clicked()
                {
                    let clean_name = draft.profile_name.trim();
                    let final_name = if clean_name.is_empty() {
                        format!(
                            "RGB {:02X}{:02X}{:02X}",
                            cfg.lighting.zones[0].r,
                            cfg.lighting.zones[0].g,
                            cfg.lighting.zones[0].b
                        )
                    } else {
                        clean_name.to_string()
                    };
                    let candidate = unique_profile_slug(&final_name, "rgb", |id| {
                        cfg.lighting.custom_profiles.iter().any(|p| p.id == id)
                            || LightingProfile::builtins().iter().any(|p| p.id == id)
                    });
                    let prof = LightingProfile {
                        id: candidate,
                        name: final_name,
                        brightness_pct: cfg.lighting.brightness_pct,
                        zones: cfg.lighting.zones,
                        builtin: false,
                    };
                    apply_lighting_profile(cfg, &prof);
                    cfg.lighting.custom_profiles.push(prof);
                    draft.profile_name.clear();
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyLighting);
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
                warn_c,
                RichText::new(i18n.t_fmt(
                    "gpu.mux_confirm_prompt",
                    &[("mode", &format!("{pending:?}"))],
                ))
                .strong(),
            );
            ui.colored_label(
                err_c,
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
            ui.label(RichText::new(i18n.t("gpu.reboot_required")).color(warn_c));
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
            ui.colored_label(warn_c, i18n.t("tuning.admin_required_warning"));
        }
    });
}

pub fn apply_lighting_profile(cfg: &mut AppConfig, profile: &LightingProfile) {
    cfg.lighting.zones = profile.zones;
    cfg.lighting.brightness_pct = profile.brightness_pct;
    cfg.lighting.enabled = true;
    cfg.lighting.active_profile_id = Some(profile.id.clone());
}

pub fn is_lighting_profile_active(cfg: &AppConfig, profile: &LightingProfile) -> bool {
    let base_match = cfg.lighting.enabled
        && cfg.lighting.brightness_pct == profile.brightness_pct
        && cfg.lighting.zones == profile.zones;
    if let Some(ref active_id) = cfg.lighting.active_profile_id {
        return base_match && active_id == &profile.id;
    }
    base_match
}
