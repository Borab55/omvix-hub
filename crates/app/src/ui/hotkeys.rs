use crate::theme::{
    accent_color, card_frame, pill_button, section_title, section_toggle_button, status_badge,
    status_err, status_ok, status_warn,
};
use crate::worker::WorkerCommand;
use egui::{Color32, CornerRadius, Key, Modifiers, RichText, Stroke, Ui};
use omvix_core::config::{is_valid_custom_command_syntax, OmenKeyAction};
use omvix_core::{AppConfig, I18n};

#[cfg(windows)]
use omvix_windows::hotkeys::parse_shortcut;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HotkeyBindStatus {
    #[default]
    Disabled,
    Bound(String),
    Conflict(String),
    Invalid(String),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct HotkeysDraftState {
    pub capturing_shortcut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutCaptureResult {
    None,
    Preview(String),
    Captured(String),
    Cancelled,
}

fn format_modifiers(modifiers: Modifiers) -> Vec<&'static str> {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.mac_cmd || modifiers.command && !modifiers.ctrl {
        parts.push("Win");
    }
    parts
}

fn key_to_shortcut_token(key: Key) -> Option<&'static str> {
    match key {
        Key::A => Some("A"),
        Key::B => Some("B"),
        Key::C => Some("C"),
        Key::D => Some("D"),
        Key::E => Some("E"),
        Key::F => Some("F"),
        Key::G => Some("G"),
        Key::H => Some("H"),
        Key::I => Some("I"),
        Key::J => Some("J"),
        Key::K => Some("K"),
        Key::L => Some("L"),
        Key::M => Some("M"),
        Key::N => Some("N"),
        Key::O => Some("O"),
        Key::P => Some("P"),
        Key::Q => Some("Q"),
        Key::R => Some("R"),
        Key::S => Some("S"),
        Key::T => Some("T"),
        Key::U => Some("U"),
        Key::V => Some("V"),
        Key::W => Some("W"),
        Key::X => Some("X"),
        Key::Y => Some("Y"),
        Key::Z => Some("Z"),
        Key::Num0 => Some("0"),
        Key::Num1 => Some("1"),
        Key::Num2 => Some("2"),
        Key::Num3 => Some("3"),
        Key::Num4 => Some("4"),
        Key::Num5 => Some("5"),
        Key::Num6 => Some("6"),
        Key::Num7 => Some("7"),
        Key::Num8 => Some("8"),
        Key::Num9 => Some("9"),
        Key::F1 => Some("F1"),
        Key::F2 => Some("F2"),
        Key::F3 => Some("F3"),
        Key::F4 => Some("F4"),
        Key::F5 => Some("F5"),
        Key::F6 => Some("F6"),
        Key::F7 => Some("F7"),
        Key::F8 => Some("F8"),
        Key::F9 => Some("F9"),
        Key::F10 => Some("F10"),
        Key::F11 => Some("F11"),
        Key::F12 => Some("F12"),
        Key::F13 => Some("F13"),
        Key::F14 => Some("F14"),
        Key::F15 => Some("F15"),
        Key::F16 => Some("F16"),
        Key::F17 => Some("F17"),
        Key::F18 => Some("F18"),
        Key::F19 => Some("F19"),
        Key::F20 => Some("F20"),
        Key::F21 => Some("F21"),
        Key::F22 => Some("F22"),
        Key::F23 => Some("F23"),
        Key::F24 => Some("F24"),
        Key::Space => Some("Space"),
        Key::Tab => Some("Tab"),
        Key::Home => Some("Home"),
        Key::End => Some("End"),
        Key::Insert => Some("Insert"),
        Key::Delete => Some("Delete"),
        _ => None,
    }
}

/// Evaluates simultaneous key/modifier presses into a canonical hotkey specification
/// (e.g. `"Ctrl+Shift+O"` or `"Alt+F12"`) when capture mode is active.
pub fn evaluate_shortcut_capture(
    events: &[egui::Event],
    held_modifiers: Modifiers,
) -> ShortcutCaptureResult {
    for ev in events {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = ev
        {
            if *key == Key::Escape {
                return ShortcutCaptureResult::Cancelled;
            }
            if let Some(token) = key_to_shortcut_token(*key) {
                let mut parts = format_modifiers(*modifiers);
                parts.push(token);
                return ShortcutCaptureResult::Captured(parts.join("+"));
            }
        }
    }

    let mods = format_modifiers(held_modifiers);
    if !mods.is_empty() {
        return ShortcutCaptureResult::Preview(format!("{}+...", mods.join("+")));
    }

    ShortcutCaptureResult::None
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    i18n: &I18n,
    bind_status: &HotkeyBindStatus,
    config_warnings: &[String],
    draft: &mut HotkeysDraftState,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
    on_rebind_hotkeys: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);
    let ok_c = status_ok(t);
    let warn_c = status_warn(t);
    let err_c = status_err(t);

    ui.heading(i18n.t("nav.hotkeys"));
    ui.add_space(6.0);

    // Omen Key Card
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if section_toggle_button(
                ui,
                &i18n.t("hotkeys.omen_key"),
                &mut cfg.hotkeys.capture_omen_key,
                &i18n.t("common.enabled"),
                t,
            )
            .changed()
            {
                on_save();
                on_rebind_hotkeys();
            }
        });
        ui.add_space(4.0);

        ui.add_enabled_ui(cfg.hotkeys.capture_omen_key, |ui| {
            for (action, key) in [
                (OmenKeyAction::ToggleWindow, "hotkeys.action_toggle_window"),
                (
                    OmenKeyAction::CyclePerformanceMode,
                    "hotkeys.action_cycle_mode",
                ),
                (
                    OmenKeyAction::LaunchCustomCommand,
                    "hotkeys.action_custom_app",
                ),
            ] {
                if ui
                    .radio_value(&mut cfg.hotkeys.omen_key_action, action, i18n.t(key))
                    .changed()
                {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            }

            if cfg.hotkeys.omen_key_action == OmenKeyAction::LaunchCustomCommand {
                if !config_warnings.is_empty() {
                    ui.colored_label(
                        warn_c,
                        RichText::new(config_warnings.join(" | ")).size(11.5),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n.t("hotkeys.custom_command"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut cfg.hotkeys.custom_command)
                                .desired_width(210.0)
                                .clip_text(true),
                        )
                        .changed()
                    {
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                    let cmd_trimmed = cfg.hotkeys.custom_command.trim();
                    let cmd_valid =
                        !cmd_trimmed.is_empty() && is_valid_custom_command_syntax(cmd_trimmed);
                    if !cmd_trimmed.is_empty() {
                        if cmd_valid {
                            status_badge(ui, &i18n.t("common.status_ok"), ok_c);
                        } else {
                            status_badge(ui, &i18n.t("common.status_error"), err_c);
                        }
                    }
                    if ui
                        .add_enabled(cmd_valid, egui::Button::new(i18n.t("hotkeys.test_command")))
                        .clicked()
                    {
                        on_cmd(WorkerCommand::LaunchCustomCommand {
                            command: cfg.hotkeys.custom_command.clone(),
                            args: cfg.hotkeys.custom_args.clone(),
                        });
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(i18n.t("hotkeys.custom_args"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut cfg.hotkeys.custom_args)
                                .desired_width(210.0)
                                .clip_text(true),
                        )
                        .changed()
                    {
                        cfg.hotkeys
                            .custom_args
                            .retain(|c| !matches!(c, '\0' | '\r' | '\n'));
                        on_save();
                        on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    }
                });
                ui.weak(RichText::new(i18n.t("hotkeys.custom_command_hint")).size(11.0));
            }
        });
    });

    ui.add_space(8.0);

    // Global Shortcut Card
    card_frame(t).show(ui, |ui| {
        section_title(ui, i18n.t("hotkeys.global_shortcut"), t);
        ui.horizontal_wrapped(|ui| {
            let (badge_text, badge_color) = match bind_status {
                HotkeyBindStatus::Disabled => {
                    (i18n.t("common.disabled"), ui.visuals().weak_text_color())
                }
                HotkeyBindStatus::Bound(_) => (i18n.t("hotkeys.status_active"), ok_c),
                HotkeyBindStatus::Conflict(_) => (i18n.t("hotkeys.status_conflict"), warn_c),
                HotkeyBindStatus::Invalid(_) | HotkeyBindStatus::Error(_) => {
                    (i18n.t("common.status_error"), err_c)
                }
            };
            status_badge(ui, &badge_text, badge_color);
            if let HotkeyBindStatus::Bound(combo) = bind_status {
                ui.weak(combo);
            }
        });
        ui.add_space(6.0);
        let mut live_preview: Option<String> = None;
        if draft.capturing_shortcut {
            let cap_res = ui.input(|i| evaluate_shortcut_capture(&i.events, i.modifiers));
            match cap_res {
                ShortcutCaptureResult::Captured(combo) => {
                    cfg.hotkeys.global_toggle_shortcut = combo;
                    draft.capturing_shortcut = false;
                    on_save();
                    on_rebind_hotkeys();
                }
                ShortcutCaptureResult::Cancelled => {
                    draft.capturing_shortcut = false;
                }
                ShortcutCaptureResult::Preview(prev) => {
                    live_preview = Some(prev);
                }
                ShortcutCaptureResult::None => {}
            }
        }

        ui.horizontal_wrapped(|ui| {
            let display_text = if draft.capturing_shortcut {
                live_preview.unwrap_or_else(|| i18n.t("hotkeys.recording_prompt"))
            } else if cfg.hotkeys.global_toggle_shortcut.trim().is_empty() {
                i18n.t("hotkeys.not_set")
            } else {
                cfg.hotkeys.global_toggle_shortcut.trim().to_string()
            };

            let box_fill = if draft.capturing_shortcut {
                Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    if t.dark_base { 44 } else { 28 },
                )
            } else {
                ui.visuals().extreme_bg_color
            };
            let box_stroke = if draft.capturing_shortcut {
                Stroke::new(1.4_f32, accent)
            } else {
                Stroke::new(1.0_f32, crate::theme::rgb(t.card_border))
            };
            let box_resp = egui::Frame::NONE
                .fill(box_fill)
                .stroke(box_stroke)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.set_min_width(180.0);
                    let mut text = RichText::new(&display_text).monospace().size(13.0);
                    if draft.capturing_shortcut {
                        text =
                            text.strong()
                                .color(if t.dark_base { Color32::WHITE } else { accent });
                    }
                    ui.label(text);
                })
                .response
                .interact(egui::Sense::click());
            if box_resp.clicked() {
                draft.capturing_shortcut = !draft.capturing_shortcut;
            }

            let record_label = if draft.capturing_shortcut {
                i18n.t("common.cancel")
            } else {
                i18n.t("hotkeys.record_btn")
            };
            if pill_button(ui, &record_label, draft.capturing_shortcut, t).clicked() {
                draft.capturing_shortcut = !draft.capturing_shortcut;
            }

            if !cfg.hotkeys.global_toggle_shortcut.trim().is_empty()
                && ui.button(i18n.t("hotkeys.clear_btn")).clicked()
            {
                draft.capturing_shortcut = false;
                cfg.hotkeys.global_toggle_shortcut.clear();
                on_save();
                on_rebind_hotkeys();
            }

            #[cfg(windows)]
            if !cfg.hotkeys.global_toggle_shortcut.trim().is_empty() {
                if parse_shortcut(&cfg.hotkeys.global_toggle_shortcut).is_ok() {
                    status_badge(ui, &i18n.t("common.status_ok"), ok_c);
                } else {
                    status_badge(ui, &i18n.t("common.status_error"), err_c);
                }
            }
        });
        match bind_status {
            HotkeyBindStatus::Conflict(combo) => {
                ui.add_space(4.0);
                ui.colored_label(
                    warn_c,
                    RichText::new(
                        i18n.t("hotkeys.conflict_detail")
                            .replace("{shortcut}", combo),
                    )
                    .size(12.0),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.weak(i18n.t("hotkeys.conflict_alternatives"));
                    for alt in ["Ctrl+Alt+O", "Ctrl+Shift+F12", "Alt+F12"] {
                        if ui.small_button(alt).clicked() {
                            draft.capturing_shortcut = false;
                            cfg.hotkeys.global_toggle_shortcut = alt.to_string();
                            on_save();
                            on_rebind_hotkeys();
                        }
                    }
                });
            }
            HotkeyBindStatus::Invalid(msg) | HotkeyBindStatus::Error(msg) => {
                ui.add_space(4.0);
                ui.colored_label(err_c, RichText::new(msg).size(12.0));
            }
            HotkeyBindStatus::Disabled | HotkeyBindStatus::Bound(_) => {}
        }
        ui.add_space(4.0);
        ui.weak(i18n.t("hotkeys.capture_hint"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_shortcut_capture_formats_combinations_and_cancel() {
        let evs = vec![egui::Event::Key {
            key: Key::O,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::NONE
            },
        }];
        let res = evaluate_shortcut_capture(&evs, Modifiers::NONE);
        assert_eq!(
            res,
            ShortcutCaptureResult::Captured("Ctrl+Shift+O".to_string())
        );
        #[cfg(windows)]
        assert!(parse_shortcut("Ctrl+Shift+O").is_ok());

        let f12 = vec![egui::Event::Key {
            key: Key::F12,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
        }];
        assert_eq!(
            evaluate_shortcut_capture(&f12, Modifiers::NONE),
            ShortcutCaptureResult::Captured("Alt+F12".to_string())
        );
        #[cfg(windows)]
        assert!(parse_shortcut("Alt+F12").is_ok());

        let esc = vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }];
        assert_eq!(
            evaluate_shortcut_capture(&esc, Modifiers::NONE),
            ShortcutCaptureResult::Cancelled
        );

        let preview = evaluate_shortcut_capture(
            &[],
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::NONE
            },
        );
        assert_eq!(
            preview,
            ShortcutCaptureResult::Preview("Ctrl+Shift+...".to_string())
        );
    }
}
