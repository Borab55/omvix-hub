use crate::theme::{
    accent_color, card_frame, fixed_color_picker_button, lighting_profile_button,
    lighting_profile_chip, section_title, section_toggle_button, status_badge, ColorButtonStyle,
    SavedChipAction,
};
use crate::ui::{control_committed, unique_profile_slug};
use crate::worker::{SharedState, WorkerCommand};
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, I18n, KeyboardType, LightingProfile, RgbColor};

#[derive(Debug, Default, Clone)]
pub struct LightingDisplayDraftState {
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

    ui.heading(i18n.t("nav.lighting_display"));
    ui.add_space(6.0);

    // Keyboard Backlight & RGB Card
    card_frame(t).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
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
            let kbd_label = match st.keyboard_type {
                KeyboardType::None => i18n.t("lighting.type_none"),
                KeyboardType::WhiteBacklight => i18n.t("lighting.type_white"),
                KeyboardType::SingleZoneRgb => i18n.t("lighting.type_1zone"),
                KeyboardType::FourZoneRgb => i18n.t("lighting.type_4zone"),
                KeyboardType::PerKeyRgb => i18n.t("lighting.type_per_key"),
                KeyboardType::Unknown => i18n.t("battery.mode_unknown"),
            };
            status_badge(ui, &kbd_label, accent);
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
        let four_zone = zone_count == 4;

        if zone_count > 0 {
            ui.add_space(4.0);
            let mut zone_committed = false;
            ui.horizontal_wrapped(|ui| {
                for z in 0..zone_count {
                    let cur = cfg.lighting.zones[z];
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
                        let picked = RgbColor {
                            r: srgb[0],
                            g: srgb[1],
                            b: srgb[2],
                        };
                        if zone_count == 1 {
                            cfg.lighting.zones = [picked; 4];
                        } else {
                            cfg.lighting.zones[z] = picked;
                        }
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
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                ui.label(i18n.t("lighting.presets"));
                for preset in LightingProfile::builtins() {
                    let selected = is_lighting_profile_active(cfg, &preset);
                    if lighting_profile_button(
                        ui,
                        &preset.name,
                        &preset.zones,
                        selected,
                        four_zone,
                        t,
                    )
                    .clicked()
                    {
                        apply_lighting_profile(cfg, &preset);
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                        on_cmd(WorkerCommand::ApplyLighting);
                    }
                }
                if !cfg.lighting.custom_profiles.is_empty() {
                    let mut apply_prof: Option<LightingProfile> = None;
                    let mut delete_idx: Option<usize> = None;
                    let del_tip = i18n.t("lighting.delete_profile");
                    for (idx, prof) in cfg.lighting.custom_profiles.iter().enumerate() {
                        let is_active = is_lighting_profile_active(cfg, prof);
                        match lighting_profile_chip(
                            ui,
                            (&prof.id, idx),
                            &format!("{} ({}%)", prof.name, prof.brightness_pct),
                            &prof.zones,
                            is_active,
                            four_zone,
                            &del_tip,
                            t,
                        ) {
                            SavedChipAction::Apply => apply_prof = Some(prof.clone()),
                            SavedChipAction::Delete => delete_idx = Some(idx),
                            SavedChipAction::None => {}
                        }
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
            });

            ui.separator();

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
                    let saved_zones = if zone_count == 1 {
                        [cfg.lighting.zones[0]; 4]
                    } else {
                        cfg.lighting.zones
                    };
                    cfg.lighting.zones = saved_zones;
                    let prof = LightingProfile {
                        id: candidate,
                        name: final_name,
                        brightness_pct: cfg.lighting.brightness_pct,
                        zones: saved_zones,
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
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
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
