use crate::theme::{
    card_frame, feature_toggle_button, fixed_color_picker_button, pill_button, rgb,
    saved_theme_chip, section_title, status_err, status_ok, ColorButtonStyle, SavedChipAction,
};
use crate::ui::control_committed;
use egui::{RichText, Slider, Ui};
use omvix_core::{AppConfig, CustomThemeConfig, I18n};

#[derive(Debug, Default, Clone)]
pub struct AppearanceDraftState {
    pub import_input: String,
    pub status_msg: Option<(String, bool)>,
}

#[derive(Default)]
pub struct AppearanceActions {
    pub theme_changed: bool,
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &I18n,
    draft: &mut AppearanceDraftState,
    on_save: &mut dyn FnMut(),
) -> AppearanceActions {
    let mut actions = AppearanceActions::default();
    let theme_snapshot = cfg.general.custom_theme.clone();
    let ok_c = status_ok(&theme_snapshot);
    let err_c = status_err(&theme_snapshot);

    ui.heading(i18n.t("appearance.title"));
    ui.add_space(6.0);

    // Card 1: Shape, Corner Radii & Border Thickness
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("appearance.shape_title"), &theme_snapshot);
        ui.weak(RichText::new(i18n.t("appearance.shape_desc")).size(11.0));
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            for (key, card_r, ctrl_r, bw_x10) in [
                ("appearance.preset_sharp", 0u8, 0u8, 10u8),
                ("appearance.preset_modern", 6u8, 5u8, 10u8),
                ("appearance.preset_soft", 10u8, 8u8, 10u8),
            ] {
                let selected = cfg.general.custom_theme.card_radius == card_r
                    && cfg.general.custom_theme.control_radius == ctrl_r;
                if pill_button(ui, &i18n.t(key), selected, &theme_snapshot).clicked() {
                    cfg.general.custom_theme.card_radius = card_r;
                    cfg.general.custom_theme.control_radius = ctrl_r;
                    if !cfg.general.custom_theme.is_high_contrast() {
                        cfg.general.custom_theme.border_width_x10 = bw_x10;
                    }
                    actions.theme_changed = true;
                    on_save();
                }
            }
            if ui.button(i18n.t("appearance.reset_shape")).clicked() {
                cfg.general.custom_theme.card_radius = 6;
                cfg.general.custom_theme.control_radius = 5;
                cfg.general.custom_theme.border_width_x10 =
                    if cfg.general.custom_theme.is_high_contrast() {
                        15
                    } else {
                        10
                    };
                actions.theme_changed = true;
                on_save();
            }
        });

        ui.add_space(6.0);
        egui::Grid::new("appearance_geometry_grid")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label(i18n.t("appearance.card_radius"));
                let card_resp = ui.add(
                    Slider::new(&mut cfg.general.custom_theme.card_radius, 0..=14).suffix(" px"),
                );
                if card_resp.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, "appearance_card_radius", &card_resp) {
                    on_save();
                }
                ui.end_row();

                ui.label(i18n.t("appearance.control_radius"));
                let ctrl_resp = ui.add(
                    Slider::new(&mut cfg.general.custom_theme.control_radius, 0..=12).suffix(" px"),
                );
                if ctrl_resp.changed() {
                    actions.theme_changed = true;
                }
                if control_committed(ui, "appearance_ctrl_radius", &ctrl_resp) {
                    on_save();
                }
                ui.end_row();

                ui.label(i18n.t("appearance.border_width"));
                let mut border_px =
                    f32::from(cfg.general.custom_theme.border_width_x10.clamp(5, 25)) / 10.0;
                let bw_resp = ui.add(
                    Slider::new(&mut border_px, 0.5..=2.5)
                        .step_by(0.1)
                        .fixed_decimals(1)
                        .suffix(" px"),
                );
                if bw_resp.changed() {
                    cfg.general.custom_theme.border_width_x10 =
                        ((border_px * 10.0).round() as u8).clamp(5, 25);
                    actions.theme_changed = true;
                }
                if control_committed(ui, "appearance_border_w", &bw_resp) {
                    on_save();
                }
                ui.end_row();
            });
    });

    ui.add_space(8.0);

    // Card 2: Accessibility & Typography
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("settings.accessibility"), &theme_snapshot);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.font_scale"));
            let scale_resp = ui.add(
                Slider::new(&mut cfg.general.font_scale_pct, 85..=130)
                    .suffix("%")
                    .step_by(5.0),
            );
            if control_committed(ui, "appearance_font_scale", &scale_resp) {
                actions.theme_changed = true;
                on_save();
            }
            for pct in [85u8, 100, 115, 130] {
                if ui
                    .selectable_label(cfg.general.font_scale_pct == pct, format!("{pct}%"))
                    .clicked()
                {
                    cfg.general.font_scale_pct = pct;
                    actions.theme_changed = true;
                    on_save();
                }
            }
        });
        ui.add_space(4.0);
        let mut high_contrast = cfg.general.custom_theme.is_high_contrast();
        if feature_toggle_button(
            ui,
            &i18n.t("settings.high_contrast_mode"),
            &mut high_contrast,
            &i18n.t("common.enabled"),
            &theme_snapshot,
        )
        .changed()
        {
            cfg.general.custom_theme = if high_contrast {
                CustomThemeConfig::preset_high_contrast()
            } else {
                CustomThemeConfig::preset_dark_slate()
            };
            cfg.general.theme = "dark".to_string();
            actions.theme_changed = true;
            on_save();
        }
        ui.add_space(4.0);
        ui.weak(i18n.t("settings.keyboard_nav_hint"));
    });

    ui.add_space(8.0);

    // Card 3: Custom Theme Colors, Presets, Saved Themes & Share Codes
    card_frame(&theme_snapshot).show(ui, |ui| {
        let card_w = ui.available_width().max(260.0);
        ui.set_width(card_w);
        ui.set_max_width(card_w);
        section_title(ui, i18n.t("settings.custom_theme"), &theme_snapshot);
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.theme_presets"));
            for preset in CustomThemeConfig::presets() {
                let selected = cfg.general.custom_theme == preset;
                if pill_button(ui, &preset.name, selected, &theme_snapshot).clicked() {
                    cfg.general.custom_theme = preset.clone();
                    cfg.general.theme = if preset.dark_base {
                        "dark".to_string()
                    } else {
                        "light".to_string()
                    };
                    on_save();
                    actions.theme_changed = true;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.theme_name"));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut cfg.general.custom_theme.name)
                        .desired_width((card_w * 0.34).clamp(130.0, 180.0))
                        .clip_text(true),
                )
                .changed()
            {
                on_save();
            }
            if pill_button(
                ui,
                &i18n.t("settings.save_theme_btn"),
                false,
                &theme_snapshot,
            )
            .clicked()
            {
                let mut to_save = cfg.general.custom_theme.clone();
                let clean = to_save.name.trim();
                to_save.name = if clean.is_empty() {
                    format!("Theme {}", cfg.general.custom_themes.len() + 1)
                } else {
                    clean.to_string()
                };
                if let Some(existing) = cfg
                    .general
                    .custom_themes
                    .iter_mut()
                    .find(|x| x.name.eq_ignore_ascii_case(&to_save.name))
                {
                    *existing = to_save;
                } else {
                    cfg.general.custom_themes.push(to_save);
                }
                on_save();
                draft.status_msg = Some((i18n.t("settings.theme_saved"), false));
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if feature_toggle_button(
                ui,
                &i18n.t("settings.theme_dark_base"),
                &mut cfg.general.custom_theme.dark_base,
                &i18n.t("common.enabled"),
                &theme_snapshot,
            )
            .changed()
            {
                cfg.general.theme = if cfg.general.custom_theme.dark_base {
                    "dark".to_string()
                } else {
                    "light".to_string()
                };
                on_save();
                actions.theme_changed = true;
            }
        });

        ui.add_space(4.0);
        egui::Grid::new("custom_theme_pickers_grid")
            .num_columns(4)
            .min_col_width(0.0)
            .max_col_width(((card_w - 110.0) * 0.5).max(90.0))
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                let swatch_style = ColorButtonStyle::Swatch {
                    size: egui::vec2(34.0, 20.0),
                    radius: cfg.general.custom_theme.control_radius.clamp(0, 12),
                };
                let color_fields: [(&str, &mut [u8; 3]); 12] = [
                    (
                        "settings.color_accent",
                        &mut cfg.general.custom_theme.accent,
                    ),
                    (
                        "settings.color_sidebar",
                        &mut cfg.general.custom_theme.sidebar_bg,
                    ),
                    (
                        "settings.color_canvas",
                        &mut cfg.general.custom_theme.canvas_bg,
                    ),
                    ("settings.color_card", &mut cfg.general.custom_theme.card_bg),
                    (
                        "settings.color_border",
                        &mut cfg.general.custom_theme.card_border,
                    ),
                    (
                        "settings.color_text",
                        &mut cfg.general.custom_theme.text_primary,
                    ),
                    (
                        "settings.color_text_muted",
                        &mut cfg.general.custom_theme.text_muted,
                    ),
                    (
                        "settings.color_control_bg",
                        &mut cfg.general.custom_theme.control_bg,
                    ),
                    (
                        "settings.color_control_hover",
                        &mut cfg.general.custom_theme.control_hover_bg,
                    ),
                    (
                        "settings.color_status_ok",
                        &mut cfg.general.custom_theme.status_ok,
                    ),
                    (
                        "settings.color_status_warn",
                        &mut cfg.general.custom_theme.status_warn,
                    ),
                    (
                        "settings.color_status_err",
                        &mut cfg.general.custom_theme.status_err,
                    ),
                ];
                for (idx, (label_key, field)) in color_fields.into_iter().enumerate() {
                    ui.label(i18n.t(label_key));
                    let resp = fixed_color_picker_button(
                        ui,
                        ("appearance_theme_color_picker", idx),
                        field,
                        swatch_style,
                        &theme_snapshot,
                    );
                    if resp.changed() {
                        actions.theme_changed = true;
                    }
                    if control_committed(ui, ("theme_color", idx), &resp) {
                        on_save();
                    }
                    if idx % 2 == 1 {
                        ui.end_row();
                    }
                }
            });

        ui.add_space(6.0);
        section_title(ui, i18n.t("settings.saved_themes"), &theme_snapshot);
        ui.add_space(4.0);
        if cfg.general.custom_themes.is_empty() {
            ui.weak(i18n.t("settings.no_saved_themes"));
        } else {
            let mut apply_saved: Option<CustomThemeConfig> = None;
            let mut delete_saved: Option<usize> = None;
            let del_tip = i18n.t("settings.delete_theme_tooltip");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for (idx, saved) in cfg.general.custom_themes.iter().enumerate() {
                    let active = cfg.general.custom_theme == *saved;
                    let chip_action = ui
                        .push_id(idx, |ui| {
                            saved_theme_chip(
                                ui,
                                &saved.name,
                                rgb(saved.accent),
                                active,
                                &del_tip,
                                &theme_snapshot,
                            )
                        })
                        .inner;
                    match chip_action {
                        SavedChipAction::Apply => apply_saved = Some(saved.clone()),
                        SavedChipAction::Delete => delete_saved = Some(idx),
                        SavedChipAction::None => {}
                    }
                }
            });
            if let Some(chosen) = apply_saved {
                cfg.general.custom_theme = chosen.clone();
                cfg.general.theme = if chosen.dark_base {
                    "dark".to_string()
                } else {
                    "light".to_string()
                };
                on_save();
                actions.theme_changed = true;
            }
            if let Some(idx) = delete_saved {
                cfg.general.custom_themes.remove(idx);
                on_save();
            }
        }

        ui.separator();

        let full_input_w = (card_w - 8.0).max(160.0);
        let share_code = cfg.general.custom_theme.encode_share_code();
        ui.horizontal_wrapped(|ui| {
            ui.label(i18n.t("settings.share_code"));
            if ui.button(i18n.t("settings.copy_theme")).clicked() {
                ui.ctx().copy_text(share_code.clone());
                draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
            }
            if ui.button(i18n.t("settings.copy_theme_json")).clicked() {
                if let Ok(json) = serde_json::to_string_pretty(&cfg.general.custom_theme) {
                    ui.ctx().copy_text(json);
                    draft.status_msg = Some((i18n.t("settings.theme_copied"), false));
                }
            }
        });
        let mut preview = share_code;
        ui.add_enabled(
            false,
            egui::TextEdit::singleline(&mut preview)
                .desired_width(full_input_w)
                .clip_text(true),
        );

        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut draft.import_input)
                .hint_text(i18n.t("settings.import_placeholder"))
                .desired_width(full_input_w)
                .clip_text(true),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(RichText::new(i18n.t("settings.import_theme")).strong())
                .clicked()
            {
                match CustomThemeConfig::decode_share_code(&draft.import_input) {
                    Ok(imported) => {
                        cfg.general.custom_theme = imported.clone();
                        cfg.general.theme = if imported.dark_base {
                            "dark".to_string()
                        } else {
                            "light".to_string()
                        };
                        on_save();
                        actions.theme_changed = true;
                        draft.import_input.clear();
                        draft.status_msg = Some((i18n.t("settings.theme_imported"), false));
                    }
                    Err(err) => {
                        draft.status_msg = Some((err.to_string(), true));
                    }
                }
            }
        });

        if let Some((msg, is_err)) = &draft.status_msg {
            ui.colored_label(if *is_err { err_c } else { ok_c }, msg);
        }
    });

    actions
}
