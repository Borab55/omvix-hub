use crate::theme::{
    STATUS_ERR, STATUS_OK, accent_color, card_frame, pill_button, section_title, status_badge,
};
use crate::worker::{SharedState, WorkerCommand};
use egui::{Color32, RichText, Ui};
use omvix_core::app_profiles::{AppProfileRule, normalize_exe_name};
use omvix_core::{
    AppConfig, CpuBoostMode, FanCurvePreset, FanMode, I18n, PerformanceMode,
};

#[derive(Debug, Clone)]
pub struct ProfilesDraftState {
    pub new_name: String,
    pub new_exe: String,
    pub new_perf_mode: PerformanceMode,
    pub new_fan_mode: Option<FanMode>,
    pub new_curve_preset: Option<FanCurvePreset>,
    pub new_refresh_hz: Option<u32>,
    pub new_cpu_boost: Option<CpuBoostMode>,
    pub error_msg: Option<String>,
}

impl Default for ProfilesDraftState {
    fn default() -> Self {
        Self {
            new_name: String::new(),
            new_exe: String::new(),
            new_perf_mode: PerformanceMode::Performance,
            new_fan_mode: None,
            new_curve_preset: None,
            new_refresh_hz: None,
            new_cpu_boost: None,
            error_msg: None,
        }
    }
}

fn perf_label(i18n: &I18n, mode: PerformanceMode) -> String {
    match mode {
        PerformanceMode::Eco => i18n.t("thermal.mode_eco"),
        PerformanceMode::Quiet => i18n.t("thermal.mode_quiet"),
        PerformanceMode::Balanced => i18n.t("thermal.mode_balanced"),
        PerformanceMode::Performance => i18n.t("thermal.mode_performance"),
    }
}

fn fan_label_opt(i18n: &I18n, fan: Option<FanMode>) -> String {
    match fan {
        None => i18n.t("auto_profile.keep_current"),
        Some(FanMode::Auto) => i18n.t("fan.mode_auto"),
        Some(FanMode::Max) => i18n.t("fan.mode_max"),
        Some(FanMode::Manual) => i18n.t("fan.mode_manual"),
        Some(FanMode::CustomCurve) => i18n.t("fan.mode_curve"),
    }
}

fn curve_preset_label(i18n: &I18n, preset: FanCurvePreset) -> String {
    match preset {
        FanCurvePreset::Quiet => i18n.t("fan.preset_quiet"),
        FanCurvePreset::Balanced => i18n.t("fan.preset_balanced"),
        FanCurvePreset::Aggressive => i18n.t("fan.preset_aggressive"),
        FanCurvePreset::OghLike => i18n.t("fan.preset_ogh_like"),
    }
}

fn boost_label_opt(i18n: &I18n, boost: Option<CpuBoostMode>) -> String {
    match boost {
        None => i18n.t("auto_profile.keep_current"),
        Some(CpuBoostMode::Disabled) => i18n.t("cockpit.boost_off"),
        Some(CpuBoostMode::Enabled) => i18n.t("cockpit.boost_on"),
        Some(CpuBoostMode::Aggressive) => i18n.t("cockpit.boost_aggressive"),
        Some(CpuBoostMode::EfficientAggressive) => i18n.t("profiles.boost_efficient"),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_power_source_profile(
    ui: &mut Ui,
    id_prefix: &str,
    heading: &str,
    i18n: &I18n,
    perf_mode: &mut PerformanceMode,
    fan_mode: &mut Option<FanMode>,
    curve_preset: &mut Option<FanCurvePreset>,
    refresh_hz: &mut Option<u32>,
    supported_hz: Option<&[u32]>,
) -> bool {
    let mut changed = false;
    ui.label(RichText::new(heading).strong());
    ui.horizontal_wrapped(|ui| {
        ui.label(i18n.t("auto_profile.perf_label"));
        egui::ComboBox::from_id_salt(format!("{id_prefix}_perf"))
            .selected_text(perf_label(i18n, *perf_mode))
            .show_ui(ui, |ui| {
                for m in [
                    PerformanceMode::Eco,
                    PerformanceMode::Quiet,
                    PerformanceMode::Balanced,
                    PerformanceMode::Performance,
                ] {
                    if ui
                        .selectable_value(perf_mode, m, perf_label(i18n, m))
                        .changed()
                    {
                        changed = true;
                    }
                }
            });

        ui.add_space(6.0);
        ui.label(i18n.t("auto_profile.fan_label"));
        egui::ComboBox::from_id_salt(format!("{id_prefix}_fan"))
            .selected_text(fan_label_opt(i18n, *fan_mode))
            .show_ui(ui, |ui| {
                for opt in [
                    None,
                    Some(FanMode::Auto),
                    Some(FanMode::Max),
                    Some(FanMode::Manual),
                    Some(FanMode::CustomCurve),
                ] {
                    if ui
                        .selectable_value(fan_mode, opt, fan_label_opt(i18n, opt))
                        .changed()
                    {
                        if *fan_mode != Some(FanMode::CustomCurve) {
                            *curve_preset = None;
                        }
                        changed = true;
                    }
                }
            });

        if *fan_mode == Some(FanMode::CustomCurve) {
            ui.add_space(6.0);
            ui.label(i18n.t("auto_profile.curve_label"));
            let selected_curve_txt = curve_preset
                .map(|p| curve_preset_label(i18n, p))
                .unwrap_or_else(|| i18n.t("auto_profile.current_curve"));
            egui::ComboBox::from_id_salt(format!("{id_prefix}_curve"))
                .selected_text(selected_curve_txt)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(curve_preset, None, i18n.t("auto_profile.current_curve"))
                        .changed()
                    {
                        changed = true;
                    }
                    for &preset in FanCurvePreset::all() {
                        if ui
                            .selectable_value(
                                curve_preset,
                                Some(preset),
                                curve_preset_label(i18n, preset),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
        }

        if let Some(hz_list) = supported_hz {
            ui.add_space(6.0);
            ui.label(i18n.t("auto_profile.hz_label"));
            egui::ComboBox::from_id_salt(format!("{id_prefix}_hz"))
                .selected_text(
                    refresh_hz
                        .map(|h| i18n.t_fmt("display.hz", &[("hz", &h.to_string())]))
                        .unwrap_or_else(|| "—".into()),
                )
                .show_ui(ui, |ui| {
                    if ui.selectable_value(refresh_hz, None, "—").changed() {
                        changed = true;
                    }
                    for &hz in hz_list {
                        if ui
                            .selectable_value(
                                refresh_hz,
                                Some(hz),
                                i18n.t_fmt("display.hz", &[("hz", &hz.to_string())]),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
        }
    });
    changed
}

fn derive_rule_name_from_exe(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").unwrap_or(exe);
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => exe.to_string(),
    }
}

fn next_unique_rule_id(rules: &[AppProfileRule]) -> String {
    let mut n = rules.len() + 1;
    loop {
        let candidate = format!("rule-{n}");
        if !rules.iter().any(|r| r.id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

pub fn render(
    ui: &mut Ui,
    cfg: &mut AppConfig,
    st: &SharedState,
    i18n: &I18n,
    draft: &mut ProfilesDraftState,
    on_cmd: &mut dyn FnMut(WorkerCommand),
    on_save: &mut dyn FnMut(),
) {
    let t = &cfg.general.custom_theme.clone();
    let accent = accent_color(t);
    let supported_hz = st.display_mode.as_ref().map(|d| d.supported_hz.as_slice());

    ui.heading(i18n.t("drawer.sub_profiles"));
    ui.add_space(6.0);

    // Card 1: AC / Battery Auto-Profile
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("thermal.auto_profile"), t);
            if ui
                .checkbox(&mut cfg.auto_profile.enabled, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            match st.telemetry.ac_connected {
                Some(true) => {
                    status_badge(ui, &i18n.t("dashboard.power_source_ac"), STATUS_OK);
                }
                Some(false) => {
                    status_badge(
                        ui,
                        &i18n.t("dashboard.power_source_dc"),
                        Color32::from_rgb(245, 158, 11),
                    );
                }
                None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        cfg.auto_profile.enabled,
                        egui::Button::new(i18n.t("auto_profile.apply_now")),
                    )
                    .clicked()
                {
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                    on_cmd(WorkerCommand::ApplyAutoProfileNow);
                }
            });
        });

        ui.add_space(4.0);
        ui.add_enabled_ui(cfg.auto_profile.enabled, |ui| {
            let mut changed = false;
            changed |= render_power_source_profile(
                ui,
                "auto_ac",
                &i18n.t("thermal.ac_profile"),
                i18n,
                &mut cfg.auto_profile.ac_performance_mode,
                &mut cfg.auto_profile.ac_fan_mode,
                &mut cfg.auto_profile.ac_curve_preset,
                &mut cfg.auto_profile.ac_refresh_hz,
                supported_hz,
            );
            ui.add_space(4.0);
            changed |= render_power_source_profile(
                ui,
                "auto_dc",
                &i18n.t("thermal.dc_profile"),
                i18n,
                &mut cfg.auto_profile.dc_performance_mode,
                &mut cfg.auto_profile.dc_fan_mode,
                &mut cfg.auto_profile.dc_curve_preset,
                &mut cfg.auto_profile.dc_refresh_hz,
                supported_hz,
            );
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut cfg.auto_profile.dim_keyboard_on_battery,
                    i18n.t("auto_profile.dim_keyboard_on_battery"),
                )
                .changed()
            {
                changed = true;
            }
            if changed {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        });
    });

    ui.add_space(8.0);

    // Card 2: App-Based (.exe) Profiles
    card_frame(t).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            section_title(ui, i18n.t("profiles.app_profiles_title"), t);
            if ui
                .checkbox(&mut cfg.app_profiles.enabled, i18n.t("common.enabled"))
                .changed()
            {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
            if let Some(ref active) = st.active_app_profile {
                status_badge(
                    ui,
                    &i18n.t_fmt("profiles.active_rule_badge", &[("name", active)]),
                    STATUS_OK,
                );
            } else if cfg.app_profiles.enabled {
                status_badge(ui, &i18n.t("profiles.idle_badge"), accent);
            }
        });

        ui.weak(RichText::new(i18n.t("profiles.app_profiles_desc")).size(11.5));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(i18n.t("profiles.poll_interval"));
            for secs in [2u64, 3, 5, 10] {
                if ui
                    .selectable_label(
                        cfg.app_profiles.poll_interval_secs == secs,
                        format!("{secs}s"),
                    )
                    .clicked()
                {
                    cfg.app_profiles.poll_interval_secs = secs;
                    on_save();
                    on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                }
            }
        });

        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label(RichText::new(i18n.t("profiles.add_rule_heading")).strong());
            ui.add_space(3.0);

            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("profiles.rule_name"));
                ui.add(
                    egui::TextEdit::singleline(&mut draft.new_name)
                        .desired_width(130.0)
                        .hint_text("Cyberpunk 2077"),
                );
                ui.label(i18n.t("profiles.exe_name"));
                ui.add(
                    egui::TextEdit::singleline(&mut draft.new_exe)
                        .desired_width(150.0)
                        .hint_text("game.exe"),
                );
            });

            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("profiles.pick_running_app"));
                let mut selected_pick = draft.new_exe.clone();
                let preview = if selected_pick.trim().is_empty() {
                    i18n.t("profiles.select_process")
                } else {
                    selected_pick.clone()
                };
                egui::ComboBox::from_id_salt("app_prof_pick_running")
                    .selected_text(preview)
                    .show_ui(ui, |ui| {
                        if st.running_exes_snapshot.is_empty() {
                            ui.weak(i18n.t("profiles.no_running_snapshot"));
                        }
                        for proc_exe in &st.running_exes_snapshot {
                            if ui
                                .selectable_value(&mut selected_pick, proc_exe.clone(), proc_exe)
                                .changed()
                            {
                                draft.new_exe = selected_pick.clone();
                                if draft.new_name.trim().is_empty() {
                                    draft.new_name = derive_rule_name_from_exe(&selected_pick);
                                }
                                draft.error_msg = None;
                            }
                        }
                    });
                if ui.button(i18n.t("common.refresh")).clicked() {
                    on_cmd(WorkerCommand::RefreshRunningProcesses);
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label(i18n.t("auto_profile.perf_label"));
                egui::ComboBox::from_id_salt("app_rule_new_perf")
                    .selected_text(perf_label(i18n, draft.new_perf_mode))
                    .show_ui(ui, |ui| {
                        for m in [
                            PerformanceMode::Eco,
                            PerformanceMode::Quiet,
                            PerformanceMode::Balanced,
                            PerformanceMode::Performance,
                        ] {
                            ui.selectable_value(&mut draft.new_perf_mode, m, perf_label(i18n, m));
                        }
                    });

                ui.add_space(6.0);
                ui.label(i18n.t("auto_profile.fan_label"));
                egui::ComboBox::from_id_salt("app_rule_new_fan")
                    .selected_text(fan_label_opt(i18n, draft.new_fan_mode))
                    .show_ui(ui, |ui| {
                        for opt in [
                            None,
                            Some(FanMode::Auto),
                            Some(FanMode::Max),
                            Some(FanMode::Manual),
                            Some(FanMode::CustomCurve),
                        ] {
                            if ui
                                .selectable_value(
                                    &mut draft.new_fan_mode,
                                    opt,
                                    fan_label_opt(i18n, opt),
                                )
                                .changed()
                                && draft.new_fan_mode != Some(FanMode::CustomCurve)
                            {
                                draft.new_curve_preset = None;
                            }
                        }
                    });

                if draft.new_fan_mode == Some(FanMode::CustomCurve) {
                    ui.add_space(6.0);
                    ui.label(i18n.t("auto_profile.curve_label"));
                    let cur_txt = draft
                        .new_curve_preset
                        .map(|p| curve_preset_label(i18n, p))
                        .unwrap_or_else(|| i18n.t("auto_profile.current_curve"));
                    egui::ComboBox::from_id_salt("app_rule_new_curve")
                        .selected_text(cur_txt)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.new_curve_preset,
                                None,
                                i18n.t("auto_profile.current_curve"),
                            );
                            for &preset in FanCurvePreset::all() {
                                ui.selectable_value(
                                    &mut draft.new_curve_preset,
                                    Some(preset),
                                    curve_preset_label(i18n, preset),
                                );
                            }
                        });
                }
            });

            ui.horizontal_wrapped(|ui| {
                if let Some(hz_list) = supported_hz {
                    ui.label(i18n.t("auto_profile.hz_label"));
                    egui::ComboBox::from_id_salt("app_rule_new_hz")
                        .selected_text(
                            draft
                                .new_refresh_hz
                                .map(|h| i18n.t_fmt("display.hz", &[("hz", &h.to_string())]))
                                .unwrap_or_else(|| i18n.t("auto_profile.keep_current")),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.new_refresh_hz,
                                None,
                                i18n.t("auto_profile.keep_current"),
                            );
                            for &hz in hz_list {
                                ui.selectable_value(
                                    &mut draft.new_refresh_hz,
                                    Some(hz),
                                    i18n.t_fmt("display.hz", &[("hz", &hz.to_string())]),
                                );
                            }
                        });
                    ui.add_space(6.0);
                }

                ui.label(i18n.t("profiles.cpu_boost_label"));
                egui::ComboBox::from_id_salt("app_rule_new_boost")
                    .selected_text(boost_label_opt(i18n, draft.new_cpu_boost))
                    .show_ui(ui, |ui| {
                        for opt in [
                            None,
                            Some(CpuBoostMode::Disabled),
                            Some(CpuBoostMode::Enabled),
                            Some(CpuBoostMode::Aggressive),
                            Some(CpuBoostMode::EfficientAggressive),
                        ] {
                            ui.selectable_value(
                                &mut draft.new_cpu_boost,
                                opt,
                                boost_label_opt(i18n, opt),
                            );
                        }
                    });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill_button(ui, &i18n.t("profiles.add_rule"), true, t).clicked() {
                        match normalize_exe_name(&draft.new_exe) {
                            Some(exe_name) => {
                                let name = if draft.new_name.trim().is_empty() {
                                    derive_rule_name_from_exe(&exe_name)
                                } else {
                                    draft.new_name.trim().to_string()
                                };
                                let id = next_unique_rule_id(&cfg.app_profiles.rules);
                                cfg.app_profiles.rules.push(AppProfileRule {
                                    id,
                                    name,
                                    exe_name,
                                    enabled: true,
                                    performance_mode: draft.new_perf_mode,
                                    fan_mode: draft.new_fan_mode,
                                    curve_preset: draft.new_curve_preset,
                                    refresh_hz: draft.new_refresh_hz,
                                    cpu_boost_mode: draft.new_cpu_boost,
                                });
                                draft.new_name.clear();
                                draft.new_exe.clear();
                                draft.error_msg = None;
                                on_save();
                                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
                            }
                            None => {
                                draft.error_msg = Some(i18n.t("profiles.invalid_exe_error"));
                            }
                        }
                    }
                });
            });

            if let Some(ref err) = draft.error_msg {
                ui.colored_label(STATUS_ERR, err);
            }
        });

        ui.add_space(6.0);
        ui.label(RichText::new(i18n.t("profiles.rules_priority_heading")).strong());
        if cfg.app_profiles.rules.is_empty() {
            ui.weak(i18n.t("profiles.no_rules"));
        } else {
            let len = cfg.app_profiles.rules.len();
            let mut move_up: Option<usize> = None;
            let mut move_down: Option<usize> = None;
            let mut delete_idx: Option<usize> = None;
            let mut rules_changed = false;

            for (idx, rule) in cfg.app_profiles.rules.iter_mut().enumerate() {
                let is_active = st.active_app_profile_id.as_deref() == Some(rule.id.as_str());
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.checkbox(&mut rule.enabled, "").changed() {
                            rules_changed = true;
                        }
                        if ui.add_enabled(idx > 0, egui::Button::new("▲")).clicked() {
                            move_up = Some(idx);
                        }
                        if ui
                            .add_enabled(idx + 1 < len, egui::Button::new("▼"))
                            .clicked()
                        {
                            move_down = Some(idx);
                        }

                        ui.label(RichText::new(rule.display_label()).strong());
                        ui.monospace(&rule.exe_name);

                        if is_active {
                            status_badge(ui, &i18n.t("profiles.rule_running_badge"), STATUS_OK);
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("×").clicked() {
                                delete_idx = Some(idx);
                            }
                        });
                    });

                    let mut tags = vec![perf_label(i18n, rule.performance_mode)];
                    if let Some(fm) = rule.fan_mode {
                        let fan_str = match (fm, rule.curve_preset) {
                            (FanMode::CustomCurve, Some(preset)) => format!(
                                "{} ({})",
                                fan_label_opt(i18n, Some(fm)),
                                curve_preset_label(i18n, preset)
                            ),
                            _ => fan_label_opt(i18n, Some(fm)),
                        };
                        tags.push(fan_str);
                    }
                    if let Some(hz) = rule.refresh_hz {
                        tags.push(i18n.t_fmt("display.hz", &[("hz", &hz.to_string())]));
                    }
                    if let Some(boost) = rule.cpu_boost_mode {
                        tags.push(format!("CPU: {}", boost_label_opt(i18n, Some(boost))));
                    }
                    ui.weak(RichText::new(tags.join("  ·  ")).size(11.0));
                });
            }

            if let Some(idx) = move_up {
                cfg.app_profiles.rules.swap(idx, idx - 1);
                rules_changed = true;
            }
            if let Some(idx) = move_down {
                cfg.app_profiles.rules.swap(idx, idx + 1);
                rules_changed = true;
            }
            if let Some(idx) = delete_idx {
                cfg.app_profiles.rules.remove(idx);
                rules_changed = true;
            }
            if rules_changed {
                on_save();
                on_cmd(WorkerCommand::UpdateConfig(cfg.clone()));
            }
        }
    });
}
