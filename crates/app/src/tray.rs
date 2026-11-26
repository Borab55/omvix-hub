use omvix_core::{BatteryChargeMode, FanMode, I18n, PerformanceMode};
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ToggleWindow,
    ShowAndFocus,
    SetPerformance(PerformanceMode),
    SetFan(FanMode),
    ApplyBatteryPreset {
        enabled: bool,
        stop_pct: u8,
        start_pct: u8,
    },
    Quit,
}

/// Returns `(ring_rgb, core_rgb)` accent colors reflecting the active thermal performance mode.
pub fn performance_mode_ring_rgb(mode: PerformanceMode) -> ([u8; 3], [u8; 3]) {
    match mode {
        PerformanceMode::Quiet => ([45, 212, 191], [153, 246, 228]),
        PerformanceMode::Balanced => ([0, 196, 235], [56, 224, 255]),
        PerformanceMode::Performance => ([239, 68, 68], [252, 165, 165]),
    }
}

/// Procedurally generates a crisp 32x32 RGBA Omvix emblem colored by performance mode,
/// optionally adding a corner amber badge when battery Hold/Inhibit charge limiting is active.
pub fn build_tray_icon_rgba_styled(
    mode: PerformanceMode,
    battery_limit_active: bool,
) -> (Vec<u8>, u32, u32) {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let (ring_rgb, core_rgb) = performance_mode_ring_rgb(mode);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let r = (dx * dx + dy * dy).sqrt();
            let idx = ((y * SIZE + x) * 4) as usize;

            if r <= 14.8 {
                let edge_alpha = ((14.8 - r).clamp(0.0, 1.0) * 255.0) as u8;

                if (9.3..=12.6).contains(&r) {
                    rgba[idx] = ring_rgb[0];
                    rgba[idx + 1] = ring_rgb[1];
                    rgba[idx + 2] = ring_rgb[2];
                    rgba[idx + 3] = edge_alpha;
                } else if r <= 4.3 {
                    rgba[idx] = core_rgb[0];
                    rgba[idx + 1] = core_rgb[1];
                    rgba[idx + 2] = core_rgb[2];
                    rgba[idx + 3] = edge_alpha;
                } else {
                    rgba[idx] = 16;
                    rgba[idx + 1] = 24;
                    rgba[idx + 2] = 36;
                    rgba[idx + 3] = edge_alpha;
                }
            }

            if battery_limit_active {
                let bdx = x as f32 - 24.5;
                let bdy = y as f32 - 24.5;
                let bd = (bdx * bdx + bdy * bdy).sqrt();
                if bd <= 6.2 {
                    if bd <= 4.5 {
                        rgba[idx] = 245;
                        rgba[idx + 1] = 158;
                        rgba[idx + 2] = 11;
                        rgba[idx + 3] = 255;
                    } else {
                        rgba[idx] = 18;
                        rgba[idx + 1] = 22;
                        rgba[idx + 2] = 28;
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
    }

    (rgba, SIZE, SIZE)
}

pub fn build_tray_icon_rgba() -> (Vec<u8>, u32, u32) {
    build_tray_icon_rgba_styled(PerformanceMode::Balanced, false)
}

pub struct TrayController {
    _tray: TrayIcon,
    item_toggle: MenuItem,
    sub_perf: Submenu,
    item_mode_quiet: CheckMenuItem,
    item_mode_balanced: CheckMenuItem,
    item_mode_perf: CheckMenuItem,
    sub_fan: Submenu,
    item_fan_auto: CheckMenuItem,
    item_fan_max: CheckMenuItem,
    item_fan_manual: CheckMenuItem,
    item_fan_curve: CheckMenuItem,
    sub_bat: Submenu,
    item_bat_60: CheckMenuItem,
    item_bat_80: CheckMenuItem,
    item_bat_90: CheckMenuItem,
    item_bat_100: CheckMenuItem,
    item_quit: MenuItem,
    last_icon_state: Option<(PerformanceMode, BatteryChargeMode)>,
    last_checked_state: Option<(PerformanceMode, Option<FanMode>, bool, u8)>,
}

impl TrayController {
    pub fn new(i18n: &I18n) -> Result<Self, String> {
        let (rgba, w, h) = build_tray_icon_rgba_styled(PerformanceMode::Balanced, false);
        let icon = Icon::from_rgba(rgba, w, h).map_err(|e| format!("Tray icon error: {e}"))?;

        let item_toggle =
            MenuItem::with_id(MenuId::new("toggle"), i18n.t("app.tray.show"), true, None);

        let item_mode_quiet = CheckMenuItem::with_id(
            MenuId::new("mode_quiet"),
            i18n.t("app.tray.mode_quiet"),
            true,
            false,
            None,
        );
        let item_mode_balanced = CheckMenuItem::with_id(
            MenuId::new("mode_balanced"),
            i18n.t("app.tray.mode_balanced"),
            true,
            true,
            None,
        );
        let item_mode_perf = CheckMenuItem::with_id(
            MenuId::new("mode_perf"),
            i18n.t("app.tray.mode_performance"),
            true,
            false,
            None,
        );
        let sub_perf = Submenu::new(i18n.t("thermal.mode"), true);
        let _ = sub_perf.append(&item_mode_quiet);
        let _ = sub_perf.append(&item_mode_balanced);
        let _ = sub_perf.append(&item_mode_perf);

        let item_fan_auto = CheckMenuItem::with_id(
            MenuId::new("fan_auto"),
            i18n.t("app.tray.fan_auto"),
            true,
            true,
            None,
        );
        let item_fan_max = CheckMenuItem::with_id(
            MenuId::new("fan_max"),
            i18n.t("app.tray.fan_max"),
            true,
            false,
            None,
        );
        let item_fan_manual = CheckMenuItem::with_id(
            MenuId::new("fan_manual"),
            i18n.t("app.tray.fan_manual"),
            true,
            false,
            None,
        );
        let item_fan_curve = CheckMenuItem::with_id(
            MenuId::new("fan_curve"),
            i18n.t("app.tray.fan_curve"),
            true,
            false,
            None,
        );
        let sub_fan = Submenu::new(i18n.t("fan.mode"), true);
        let _ = sub_fan.append(&item_fan_auto);
        let _ = sub_fan.append(&item_fan_max);
        let _ = sub_fan.append(&item_fan_manual);
        let _ = sub_fan.append(&item_fan_curve);

        let item_bat_60 = CheckMenuItem::with_id(
            MenuId::new("bat_60"),
            i18n.t("battery.preset_desktop"),
            true,
            false,
            None,
        );
        let item_bat_80 = CheckMenuItem::with_id(
            MenuId::new("bat_80"),
            i18n.t("battery.preset_balanced"),
            true,
            false,
            None,
        );
        let item_bat_90 = CheckMenuItem::with_id(
            MenuId::new("bat_90"),
            i18n.t("battery.preset_travel"),
            true,
            false,
            None,
        );
        let item_bat_100 = CheckMenuItem::with_id(
            MenuId::new("bat_100"),
            i18n.t("battery.preset_full"),
            true,
            true,
            None,
        );
        let sub_bat = Submenu::new(i18n.t("battery.presets_title"), true);
        let _ = sub_bat.append(&item_bat_60);
        let _ = sub_bat.append(&item_bat_80);
        let _ = sub_bat.append(&item_bat_90);
        let _ = sub_bat.append(&item_bat_100);

        let item_quit = MenuItem::with_id(MenuId::new("quit"), i18n.t("app.tray.quit"), true, None);

        let menu = Menu::new();
        let _ = menu.append(&item_toggle);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&sub_perf);
        let _ = menu.append(&sub_fan);
        let _ = menu.append(&sub_bat);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&item_quit);

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Omvix Hub")
            .with_icon(icon)
            .build()
            .map_err(|e| format!("Failed to build tray icon: {e}"))?;

        Ok(Self {
            _tray: tray,
            item_toggle,
            sub_perf,
            item_mode_quiet,
            item_mode_balanced,
            item_mode_perf,
            sub_fan,
            item_fan_auto,
            item_fan_max,
            item_fan_manual,
            item_fan_curve,
            sub_bat,
            item_bat_60,
            item_bat_80,
            item_bat_90,
            item_bat_100,
            item_quit,
            last_icon_state: None,
            last_checked_state: None,
        })
    }

    pub fn invalidate_checked_cache(&mut self) {
        self.last_checked_state = None;
    }

    pub fn update_visual_state(
        &mut self,
        perf_mode: PerformanceMode,
        fan_mode: Option<FanMode>,
        bat_enabled: bool,
        bat_stop_pct: u8,
        bat_hw_mode: BatteryChargeMode,
        i18n: &I18n,
    ) {
        let icon_key = (perf_mode, bat_hw_mode);
        if self.last_icon_state != Some(icon_key) {
            self.last_icon_state = Some(icon_key);
            let active = matches!(
                bat_hw_mode,
                BatteryChargeMode::Hold | BatteryChargeMode::Inhibit
            );
            let (rgba, w, h) = build_tray_icon_rgba_styled(perf_mode, active);
            if let Ok(icon) = Icon::from_rgba(rgba, w, h) {
                let _ = self._tray.set_icon(Some(icon));
            }
            let mode_name = match perf_mode {
                PerformanceMode::Quiet => i18n.t("thermal.mode_quiet"),
                PerformanceMode::Balanced => i18n.t("thermal.mode_balanced"),
                PerformanceMode::Performance => i18n.t("thermal.mode_performance"),
            };
            let base_tip = match bat_hw_mode {
                BatteryChargeMode::Hold => i18n.t("app.tray.tooltip_hold"),
                BatteryChargeMode::Inhibit => i18n.t("app.tray.tooltip_inhibit"),
                BatteryChargeMode::Unknown => i18n.t("app.tray.tooltip_unknown"),
                BatteryChargeMode::Auto => "Omvix Hub".to_string(),
            };
            let tooltip = format!("{base_tip} [{mode_name}]");
            let _ = self._tray.set_tooltip(Some(&tooltip));
        }

        let check_key = (perf_mode, fan_mode, bat_enabled, bat_stop_pct);
        if self.last_checked_state != Some(check_key) {
            self.last_checked_state = Some(check_key);
            self.item_mode_quiet
                .set_checked(perf_mode == PerformanceMode::Quiet);
            self.item_mode_balanced
                .set_checked(perf_mode == PerformanceMode::Balanced);
            self.item_mode_perf
                .set_checked(perf_mode == PerformanceMode::Performance);

            self.item_fan_auto
                .set_checked(fan_mode == Some(FanMode::Auto));
            self.item_fan_max
                .set_checked(fan_mode == Some(FanMode::Max));
            self.item_fan_manual
                .set_checked(fan_mode == Some(FanMode::Manual));
            self.item_fan_curve
                .set_checked(fan_mode == Some(FanMode::CustomCurve));

            self.item_bat_60
                .set_checked(bat_enabled && bat_stop_pct == 60);
            self.item_bat_80
                .set_checked(bat_enabled && bat_stop_pct == 80);
            self.item_bat_90
                .set_checked(bat_enabled && bat_stop_pct == 90);
            self.item_bat_100
                .set_checked(!bat_enabled || bat_stop_pct >= 100);
        }
    }

    pub fn update_labels(&mut self, i18n: &I18n, window_visible: bool) {
        self.item_toggle.set_text(if window_visible {
            i18n.t("app.tray.hide")
        } else {
            i18n.t("app.tray.show")
        });
        self.sub_perf.set_text(i18n.t("thermal.mode"));
        self.item_mode_quiet.set_text(i18n.t("app.tray.mode_quiet"));
        self.item_mode_balanced
            .set_text(i18n.t("app.tray.mode_balanced"));
        self.item_mode_perf
            .set_text(i18n.t("app.tray.mode_performance"));

        self.sub_fan.set_text(i18n.t("fan.mode"));
        self.item_fan_auto.set_text(i18n.t("app.tray.fan_auto"));
        self.item_fan_max.set_text(i18n.t("app.tray.fan_max"));
        self.item_fan_manual.set_text(i18n.t("app.tray.fan_manual"));
        self.item_fan_curve.set_text(i18n.t("app.tray.fan_curve"));

        self.sub_bat.set_text(i18n.t("battery.presets_title"));
        self.item_bat_60.set_text(i18n.t("battery.preset_desktop"));
        self.item_bat_80.set_text(i18n.t("battery.preset_balanced"));
        self.item_bat_90.set_text(i18n.t("battery.preset_travel"));
        self.item_bat_100.set_text(i18n.t("battery.preset_full"));

        self.item_quit.set_text(i18n.t("app.tray.quit"));
        self.last_icon_state = None;
    }

    pub fn poll_actions() -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                "toggle" => actions.push(TrayAction::ToggleWindow),
                "mode_quiet" => actions.push(TrayAction::SetPerformance(PerformanceMode::Quiet)),
                "mode_balanced" => {
                    actions.push(TrayAction::SetPerformance(PerformanceMode::Balanced))
                }
                "mode_perf" => {
                    actions.push(TrayAction::SetPerformance(PerformanceMode::Performance))
                }
                "fan_auto" => actions.push(TrayAction::SetFan(FanMode::Auto)),
                "fan_max" => actions.push(TrayAction::SetFan(FanMode::Max)),
                "fan_manual" => actions.push(TrayAction::SetFan(FanMode::Manual)),
                "fan_curve" => actions.push(TrayAction::SetFan(FanMode::CustomCurve)),
                "bat_60" => actions.push(TrayAction::ApplyBatteryPreset {
                    enabled: true,
                    stop_pct: 60,
                    start_pct: 55,
                }),
                "bat_80" => actions.push(TrayAction::ApplyBatteryPreset {
                    enabled: true,
                    stop_pct: 80,
                    start_pct: 75,
                }),
                "bat_90" => actions.push(TrayAction::ApplyBatteryPreset {
                    enabled: true,
                    stop_pct: 90,
                    start_pct: 85,
                }),
                "bat_100" => actions.push(TrayAction::ApplyBatteryPreset {
                    enabled: false,
                    stop_pct: 100,
                    start_pct: 95,
                }),
                "quit" => actions.push(TrayAction::Quit),
                _ => {}
            }
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let Some(act) = classify_tray_icon_event(&ev) {
                actions.push(act);
            }
        }

        actions
    }
}

pub(crate) fn classify_tray_icon_event(ev: &TrayIconEvent) -> Option<TrayAction> {
    match ev {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => Some(TrayAction::ShowAndFocus),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tray_icon::{dpi::PhysicalPosition, Rect, TrayIconId};

    #[test]
    fn tray_icon_click_classifier_only_triggers_show_and_focus_on_left_up() {
        let pos = PhysicalPosition::new(0.0, 0.0);
        let rect = Rect::default();
        let down = TrayIconEvent::Click {
            id: TrayIconId::new("t"),
            position: pos,
            rect,
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
        };
        assert_eq!(classify_tray_icon_event(&down), None);

        let up = TrayIconEvent::Click {
            id: TrayIconId::new("t"),
            position: pos,
            rect,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
        };
        assert_eq!(
            classify_tray_icon_event(&up),
            Some(TrayAction::ShowAndFocus)
        );

        let dbl = TrayIconEvent::DoubleClick {
            id: TrayIconId::new("t"),
            position: pos,
            rect,
            button: MouseButton::Left,
        };
        assert_eq!(classify_tray_icon_event(&dbl), None);

        let right = TrayIconEvent::Click {
            id: TrayIconId::new("t"),
            position: pos,
            rect,
            button: MouseButton::Right,
            button_state: MouseButtonState::Up,
        };
        assert_eq!(classify_tray_icon_event(&right), None);
    }

    #[test]
    fn tray_rgba_has_expected_dimensions_and_mode_colors() {
        let ring_idx = ((4 * 32 + 16) * 4) as usize; // radius ~11.5 -> on the outer ring
        let corner_idx = ((24 * 32 + 24) * 4) as usize;

        for mode in [
            PerformanceMode::Quiet,
            PerformanceMode::Balanced,
            PerformanceMode::Performance,
        ] {
            let (rgba, w, h) = build_tray_icon_rgba_styled(mode, false);
            assert_eq!(w, 32);
            assert_eq!(h, 32);
            assert_eq!(rgba.len(), 32 * 32 * 4);
            let expected_ring = performance_mode_ring_rgb(mode).0;
            assert_eq!(&rgba[ring_idx..ring_idx + 3], &expected_ring);
            assert_ne!(&rgba[corner_idx..corner_idx + 4], &[245, 158, 11, 255]);
        }

        let (badged, bw, bh) = build_tray_icon_rgba_styled(PerformanceMode::Performance, true);
        assert_eq!(bw, 32);
        assert_eq!(bh, 32);
        assert_eq!(
            &badged[ring_idx..ring_idx + 3],
            &performance_mode_ring_rgb(PerformanceMode::Performance).0
        );
        assert_eq!(&badged[corner_idx..corner_idx + 4], &[245, 158, 11, 255]);
    }
}
