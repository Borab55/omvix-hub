use omvix_core::{BatteryChargeMode, FanMode, I18n, PerformanceMode};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ToggleWindow,
    SetPerformance(PerformanceMode),
    SetFan(FanMode),
    ToggleBatteryLimit,
    Quit,
}

/// Procedurally generates a crisp 32x32 RGBA Omvix emblem (dark slate circle + luminous cyan ring).
pub fn build_tray_icon_rgba() -> (Vec<u8>, u32, u32) {
    build_tray_icon_rgba_with_badge(false)
}

/// Procedurally generates the 32x32 tray emblem, optionally adding a corner amber badge
/// while battery Hold (`0x0300`) or Inhibit (`0x0100`) charge limiting is active (R5).
pub fn build_tray_icon_rgba_with_badge(battery_limit_active: bool) -> (Vec<u8>, u32, u32) {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as f32 - 1.0) / 2.0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let r = (dx * dx + dy * dy).sqrt();
            let idx = ((y * SIZE + x) * 4) as usize;

            if r <= 14.8 {
                // Smooth outer edge alpha
                let edge_alpha = ((14.8 - r).clamp(0.0, 1.0) * 255.0) as u8;

                // Cyan outer ring [9.5 .. 13.0] and glowing core [0 .. 4.2]
                if (9.3..=12.6).contains(&r) {
                    rgba[idx] = 0;
                    rgba[idx + 1] = 196;
                    rgba[idx + 2] = 235;
                    rgba[idx + 3] = edge_alpha;
                } else if r <= 4.3 {
                    rgba[idx] = 56;
                    rgba[idx + 1] = 224;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = edge_alpha;
                } else {
                    // Deep slate background
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

pub struct TrayController {
    _tray: TrayIcon,
    item_toggle: MenuItem,
    item_mode_eco: MenuItem,
    item_mode_balanced: MenuItem,
    item_mode_perf: MenuItem,
    item_fan_auto: MenuItem,
    item_fan_max: MenuItem,
    item_bat_toggle: MenuItem,
    item_quit: MenuItem,
    battery_badge_mode: Option<BatteryChargeMode>,
}

impl TrayController {
    pub fn new(i18n: &I18n) -> Result<Self, String> {
        let (rgba, w, h) = build_tray_icon_rgba_with_badge(false);
        let icon = Icon::from_rgba(rgba, w, h).map_err(|e| format!("Tray icon error: {e}"))?;

        let item_toggle = MenuItem::with_id(MenuId::new("toggle"), i18n.t("app.tray.show"), true, None);
        let item_mode_eco =
            MenuItem::with_id(MenuId::new("mode_eco"), i18n.t("app.tray.mode_eco"), true, None);
        let item_mode_balanced = MenuItem::with_id(
            MenuId::new("mode_balanced"),
            i18n.t("app.tray.mode_balanced"),
            true,
            None,
        );
        let item_mode_perf = MenuItem::with_id(
            MenuId::new("mode_perf"),
            i18n.t("app.tray.mode_performance"),
            true,
            None,
        );
        let item_fan_auto =
            MenuItem::with_id(MenuId::new("fan_auto"), i18n.t("app.tray.fan_auto"), true, None);
        let item_fan_max =
            MenuItem::with_id(MenuId::new("fan_max"), i18n.t("app.tray.fan_max"), true, None);
        let item_bat_toggle = MenuItem::with_id(
            MenuId::new("bat_toggle"),
            i18n.t("app.tray.toggle_battery_limit"),
            true,
            None,
        );
        let item_quit = MenuItem::with_id(MenuId::new("quit"), i18n.t("app.tray.quit"), true, None);

        let menu = Menu::new();
        let _ = menu.append(&item_toggle);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&item_mode_eco);
        let _ = menu.append(&item_mode_balanced);
        let _ = menu.append(&item_mode_perf);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&item_fan_auto);
        let _ = menu.append(&item_fan_max);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&item_bat_toggle);
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
            item_mode_eco,
            item_mode_balanced,
            item_mode_perf,
            item_fan_auto,
            item_fan_max,
            item_bat_toggle,
            item_quit,
            battery_badge_mode: None,
        })
    }

    pub fn update_battery_badge(&mut self, mode: BatteryChargeMode, i18n: &I18n) {
        if self.battery_badge_mode == Some(mode) {
            return;
        }
        self.battery_badge_mode = Some(mode);
        let active = mode != BatteryChargeMode::Auto;
        let (rgba, w, h) = build_tray_icon_rgba_with_badge(active);
        if let Ok(icon) = Icon::from_rgba(rgba, w, h) {
            let _ = self._tray.set_icon(Some(icon));
        }
        let tooltip = match mode {
            BatteryChargeMode::Hold => i18n.t("app.tray.tooltip_hold"),
            BatteryChargeMode::Inhibit => i18n.t("app.tray.tooltip_inhibit"),
            BatteryChargeMode::Auto => "Omvix Hub".to_string(),
        };
        let _ = self._tray.set_tooltip(Some(&tooltip));
    }

    pub fn update_labels(&self, i18n: &I18n, window_visible: bool) {
        self.item_toggle.set_text(if window_visible {
            i18n.t("app.tray.hide")
        } else {
            i18n.t("app.tray.show")
        });
        self.item_mode_eco.set_text(i18n.t("app.tray.mode_eco"));
        self.item_mode_balanced.set_text(i18n.t("app.tray.mode_balanced"));
        self.item_mode_perf.set_text(i18n.t("app.tray.mode_performance"));
        self.item_fan_auto.set_text(i18n.t("app.tray.fan_auto"));
        self.item_fan_max.set_text(i18n.t("app.tray.fan_max"));
        self.item_bat_toggle
            .set_text(i18n.t("app.tray.toggle_battery_limit"));
        self.item_quit.set_text(i18n.t("app.tray.quit"));
    }

    pub fn poll_actions() -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                "toggle" => actions.push(TrayAction::ToggleWindow),
                "mode_eco" => actions.push(TrayAction::SetPerformance(PerformanceMode::Eco)),
                "mode_balanced" => {
                    actions.push(TrayAction::SetPerformance(PerformanceMode::Balanced))
                }
                "mode_perf" => {
                    actions.push(TrayAction::SetPerformance(PerformanceMode::Performance))
                }
                "fan_auto" => actions.push(TrayAction::SetFan(FanMode::Auto)),
                "fan_max" => actions.push(TrayAction::SetFan(FanMode::Max)),
                "bat_toggle" => actions.push(TrayAction::ToggleBatteryLimit),
                "quit" => actions.push(TrayAction::Quit),
                _ => {}
            }
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            match ev {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => actions.push(TrayAction::ToggleWindow),
                _ => {}
            }
        }

        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_rgba_has_expected_dimensions() {
        let (rgba, w, h) = build_tray_icon_rgba();
        assert_eq!(w, 32);
        assert_eq!(h, 32);
        assert_eq!(rgba.len(), 32 * 32 * 4);

        let (badged, bw, bh) = build_tray_icon_rgba_with_badge(true);
        assert_eq!(bw, 32);
        assert_eq!(bh, 32);
        let corner_idx = ((24 * 32 + 24) * 4) as usize;
        assert_eq!(&badged[corner_idx..corner_idx + 4], &[245, 158, 11, 255]);
        assert_ne!(&rgba[corner_idx..corner_idx + 4], &[245, 158, 11, 255]);
    }
}
