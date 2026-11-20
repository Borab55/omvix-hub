use crate::boards::lookup_board;
use crate::models::{PerformanceMode, ThermalProfileScheme};

/// Default maximum WMI fan level step when firmware `FanMaxGet` (0x26) is unavailable.
pub const DEFAULT_MAX_FAN_LEVEL: u8 = 55;

/// Minimum safe non-zero WMI fan level (~1800 RPM) enforced when temperatures are elevated.
pub const MIN_SAFE_FAN_LEVEL: u8 = 18;

/// Minimum fan RPM expected under warm load before the thermal guard overrides to Max Fan.
pub const MIN_GUARD_FAN_RPM: u32 = 1800;

/// Maps an abstract `PerformanceMode` and board `ThermalProfileScheme` to the HP BIOS WMI byte.
pub fn thermal_profile_byte(scheme: ThermalProfileScheme, mode: PerformanceMode) -> u8 {
    match scheme {
        ThermalProfileScheme::VictusS | ThermalProfileScheme::OmenV1 => match mode {
            PerformanceMode::Eco | PerformanceMode::Quiet => 0x50,
            PerformanceMode::Balanced => 0x30,
            PerformanceMode::Performance => 0x31,
        },
        ThermalProfileScheme::Victus
        | ThermalProfileScheme::OmenV0
        | ThermalProfileScheme::Unknown => match mode {
            PerformanceMode::Balanced => 0x00,
            PerformanceMode::Performance => 0x01,
            PerformanceMode::Eco | PerformanceMode::Quiet => 0x02,
        },
    }
}

/// Converts a 0..=100 percentage into a WMI fan level step (0..=max_level).
pub fn fan_percent_to_wmi_level(speed_pct: u8, max_level: u8) -> u8 {
    let pct = speed_pct.clamp(0, 100) as f32;
    let max_l = max_level.max(1) as f32;
    ((pct / 100.0) * max_l).round().clamp(0.0, max_l) as u8
}

/// Converts a 0..=100 percentage into a safe non-zero WMI fan level (`MIN_SAFE_FAN_LEVEL..=max_level`).
///
/// HP Victus/OMEN firmware treats level `0x00` in `0x2E` as **fan OFF (0 RPM)** rather than BIOS Auto.
pub fn safe_manual_wmi_level(speed_pct: u8, max_level: u8) -> u8 {
    let max_l = max_level.max(1);
    fan_percent_to_wmi_level(speed_pct, max_l).max(MIN_SAFE_FAN_LEVEL.min(max_l))
}

/// Returns `true` when at least one thermal sensor is available and all present sensors are cool
/// enough to safely allow 0 RPM / sub-1800 RPM silent fan curves (`CPU < 65°C`, `GPU < 60°C`, `Chassis < 46°C`).
pub fn is_system_cool_for_zero_rpm(
    cpu_c: Option<f32>,
    gpu_c: Option<f32>,
    chassis_c: Option<f32>,
) -> bool {
    let has_any = cpu_c.is_some() || gpu_c.is_some() || chassis_c.is_some();
    has_any
        && cpu_c.is_none_or(|t| t < 65.0)
        && gpu_c.is_none_or(|t| t < 60.0)
        && chassis_c.is_none_or(|t| t < 46.0)
}

/// Maps a 0..=100 fan speed percentage into a WMI fan level step, allowing 0 RPM / sub-1800 RPM
/// speeds only while the system is cool (`allow_zero_rpm == true`) and enforcing `MIN_SAFE_FAN_LEVEL`
/// otherwise.
pub fn smart_manual_wmi_level(speed_pct: u8, max_level: u8, allow_zero_rpm: bool) -> u8 {
    if allow_zero_rpm {
        fan_percent_to_wmi_level(speed_pct, max_level)
    } else {
        safe_manual_wmi_level(speed_pct, max_level)
    }
}

/// Maps HP EC chassis / IR sensor (`0x23`) readings to conservative CPU-curve thermal demand (°C).
pub fn chassis_temp_to_thermal_demand_c(chassis_c: f32) -> f32 {
    if chassis_c <= 38.0 {
        52.0
    } else if chassis_c <= 46.0 {
        52.0 + ((chassis_c - 38.0) / 8.0) * (72.0 - 52.0)
    } else if chassis_c <= 50.0 {
        72.0 + ((chassis_c - 46.0) / 4.0) * (84.0 - 72.0)
    } else if chassis_c < 54.0 {
        84.0 + ((chassis_c - 50.0) / 4.0) * (94.0 - 84.0)
    } else {
        96.0
    }
}

/// Computes the effective driving temperature (°C) for custom fan curves.
///
/// When direct CPU die telemetry (`cpu_temp_c`) is unavailable (e.g. PawnIO not installed under HVCI),
/// conservatively folds in GPU temp, mapped EC chassis demand (`0x23`), and CPU package power floor.
pub fn effective_fan_curve_temp_c(
    cpu_temp_c: Option<f32>,
    gpu_temp_c: Option<f32>,
    chassis_temp_c: Option<f32>,
    cpu_power_w: Option<f32>,
) -> Option<f32> {
    let mut temp = match (cpu_temp_c, gpu_temp_c) {
        (Some(c), Some(g)) => Some(c.max(g)),
        (Some(c), None) => Some(c),
        (None, Some(g)) => Some(g),
        (None, None) => None,
    };

    if cpu_temp_c.is_none() {
        if let Some(ch) = chassis_temp_c {
            let mapped = chassis_temp_to_thermal_demand_c(ch);
            temp = Some(temp.map_or(mapped, |t| t.max(mapped)));
        }
        let power_floor = cpu_power_w.and_then(|w| {
            if w >= 85.0 {
                Some(92.0)
            } else if w >= 65.0 {
                Some(84.0)
            } else if w >= 45.0 {
                Some(72.0)
            } else if w >= 30.0 {
                Some(62.0)
            } else {
                None
            }
        });
        if let Some(pf) = power_floor {
            temp = Some(temp.map_or(pf, |t| t.max(pf)));
        }
    } else if let Some(ch) = chassis_temp_c {
        if ch >= 50.0 {
            let mapped = chassis_temp_to_thermal_demand_c(ch);
            temp = Some(temp.map_or(mapped, |t| t.max(mapped)));
        }
    }

    temp
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThermalGuardDecision {
    NoChange,
    EngageEmergencyMax(String),
    ReleaseEmergency,
}

#[derive(Debug, Clone, Default)]
pub struct ThermalSafetyGuard {
    active: bool,
    low_rpm_polls: u8,
    hold_ticks: u8,
}

impl ThermalSafetyGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn evaluate(
        &mut self,
        cpu_c: Option<f32>,
        gpu_c: Option<f32>,
        chassis_c: Option<f32>,
        cpu_power_w: Option<f32>,
        fan_rpms: &[u32],
    ) -> ThermalGuardDecision {
        let warm_for_low_rpm = cpu_c.is_some_and(|t| t >= 78.0)
            || gpu_c.is_some_and(|t| t >= 74.0)
            || chassis_c.is_some_and(|t| t >= 48.0)
            || cpu_power_w.is_some_and(|w| w >= 55.0);
        let fans_below_safe_rpm =
            !fan_rpms.is_empty() && fan_rpms.iter().any(|&r| r < MIN_GUARD_FAN_RPM);

        self.low_rpm_polls = if warm_for_low_rpm && fans_below_safe_rpm {
            self.low_rpm_polls.saturating_add(1)
        } else {
            0
        };

        let trigger_reason = if cpu_c.is_some_and(|t| t >= 90.0) {
            Some("CPU >= 90°C".to_string())
        } else if gpu_c.is_some_and(|t| t >= 86.0) {
            Some("GPU >= 86°C".to_string())
        } else if chassis_c.is_some_and(|t| t >= 54.0) {
            Some("Chassis >= 54°C".to_string())
        } else if self.low_rpm_polls >= 2 {
            Some("Fan < 1800 RPM under thermal load".to_string())
        } else {
            None
        };

        if let Some(reason) = trigger_reason {
            self.hold_ticks = 0;
            if !self.active {
                self.active = true;
                return ThermalGuardDecision::EngageEmergencyMax(reason);
            }
            return ThermalGuardDecision::NoChange;
        }

        if self.active {
            let cooled = cpu_c.is_none_or(|t| t <= 78.0)
                && gpu_c.is_none_or(|t| t <= 74.0)
                && chassis_c.is_none_or(|t| t <= 46.0)
                && !fans_below_safe_rpm;
            if cooled {
                self.hold_ticks = self.hold_ticks.saturating_add(1);
                if self.hold_ticks >= 5 {
                    self.active = false;
                    self.hold_ticks = 0;
                    return ThermalGuardDecision::ReleaseEmergency;
                }
            } else {
                self.hold_ticks = 0;
            }
        }

        ThermalGuardDecision::NoChange
    }
}

/// Determines the most likely `ThermalProfileScheme` from board ID and model/family strings.
pub fn resolve_scheme_from_identity(
    board_id: &str,
    model: &str,
    family: &str,
) -> ThermalProfileScheme {
    if let Some(board) = lookup_board(board_id) {
        return board.thermal_scheme;
    }
    let combined = format!("{model} {family}").to_ascii_lowercase();
    if combined.contains("victus") {
        if combined.contains("16-r") || combined.contains("16-s") {
            ThermalProfileScheme::VictusS
        } else {
            ThermalProfileScheme::Victus
        }
    } else if combined.contains("omen") {
        ThermalProfileScheme::OmenV1
    } else {
        ThermalProfileScheme::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_profile_bytes_match_victus_s_and_legacy_schemes() {
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Balanced),
            0x30
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Performance),
            0x31
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Eco),
            0x50
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::OmenV1, PerformanceMode::Quiet),
            0x50
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::Victus, PerformanceMode::Balanced),
            0x00
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::Victus, PerformanceMode::Performance),
            0x01
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::OmenV0, PerformanceMode::Eco),
            0x02
        );
    }

    #[test]
    fn fan_percent_maps_cleanly_to_wmi_level() {
        assert_eq!(fan_percent_to_wmi_level(0, 55), 0);
        assert_eq!(fan_percent_to_wmi_level(100, 55), 55);
        assert_eq!(fan_percent_to_wmi_level(50, 55), 28);
        assert_eq!(fan_percent_to_wmi_level(120, 55), 55);
    }

    #[test]
    fn resolves_8c99_and_fallback_models() {
        assert_eq!(
            resolve_scheme_from_identity("8C99", "Victus 16", "HP VICTUS"),
            ThermalProfileScheme::VictusS
        );
        assert_eq!(
            resolve_scheme_from_identity("ZZZZ", "HP OMEN 16", "HP OMEN"),
            ThermalProfileScheme::OmenV1
        );
    }

    #[test]
    fn safe_manual_level_enforces_non_zero_floor() {
        assert_eq!(safe_manual_wmi_level(0, 58), MIN_SAFE_FAN_LEVEL);
        assert_eq!(safe_manual_wmi_level(10, 58), MIN_SAFE_FAN_LEVEL);
        assert_eq!(safe_manual_wmi_level(100, 58), 58);
        assert_eq!(safe_manual_wmi_level(50, 58), 29);
        assert_eq!(safe_manual_wmi_level(0, 12), 12);
    }

    #[test]
    fn cool_check_and_smart_level_allow_zero_rpm_only_when_cool() {
        assert!(is_system_cool_for_zero_rpm(Some(50.0), Some(45.0), Some(38.0)));
        assert!(is_system_cool_for_zero_rpm(None, Some(52.0), None));
        assert!(!is_system_cool_for_zero_rpm(None, None, None));
        assert!(!is_system_cool_for_zero_rpm(Some(65.0), Some(45.0), Some(38.0)));
        assert!(!is_system_cool_for_zero_rpm(Some(55.0), Some(60.0), Some(38.0)));
        assert!(!is_system_cool_for_zero_rpm(Some(55.0), Some(45.0), Some(46.0)));

        assert_eq!(smart_manual_wmi_level(0, 58, true), 0);
        assert_eq!(smart_manual_wmi_level(15, 58, true), 9);
        assert_eq!(smart_manual_wmi_level(0, 58, false), MIN_SAFE_FAN_LEVEL);
        assert_eq!(smart_manual_wmi_level(15, 58, false), MIN_SAFE_FAN_LEVEL);
        assert_eq!(smart_manual_wmi_level(60, 58, false), 35);
    }

    #[test]
    fn effective_curve_temp_bridges_chassis_and_power_when_cpu_missing() {
        let low = effective_fan_curve_temp_c(None, Some(48.0), Some(42.0), Some(18.0));
        assert!((low.unwrap() - 62.0).abs() < 0.01);

        let high_power = effective_fan_curve_temp_c(None, Some(54.0), Some(46.0), Some(90.0));
        assert_eq!(high_power, Some(92.0));

        let with_die = effective_fan_curve_temp_c(Some(68.0), Some(54.0), Some(46.0), Some(34.0));
        assert_eq!(with_die, Some(68.0));
    }

    #[test]
    fn thermal_safety_guard_engages_and_releases_with_hysteresis() {
        let mut g = ThermalSafetyGuard::new();

        // Cool idle with 0 RPM or < 1800 RPM must NOT trip the guard
        for _ in 0..4 {
            assert_eq!(
                g.evaluate(Some(52.0), Some(45.0), Some(39.0), Some(14.0), &[0, 0]),
                ThermalGuardDecision::NoChange
            );
            assert_eq!(
                g.evaluate(Some(58.0), Some(48.0), Some(41.0), Some(20.0), &[1200, 1200]),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(!g.is_active());

        // CPU >= 90°C triggers immediately even when fans are already above 1800 RPM
        assert_eq!(
            g.evaluate(Some(91.0), Some(60.0), Some(47.0), Some(55.0), &[3200, 3200]),
            ThermalGuardDecision::EngageEmergencyMax("CPU >= 90°C".to_string())
        );
        assert!(g.is_active());

        // Still warm (> 78°C) -> no release even after 6 ticks
        for _ in 0..6 {
            assert_eq!(
                g.evaluate(Some(83.0), Some(60.0), Some(45.0), Some(30.0), &[5800, 5800]),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(g.is_active());

        // Cool <= 78C and fans >= 1800 RPM -> releases after 5 consecutive cool ticks
        for _ in 0..4 {
            assert_eq!(
                g.evaluate(Some(72.0), Some(55.0), Some(43.0), Some(20.0), &[5800, 5800]),
                ThermalGuardDecision::NoChange
            );
        }
        assert_eq!(
            g.evaluate(Some(72.0), Some(55.0), Some(43.0), Some(20.0), &[5800, 5800]),
            ThermalGuardDecision::ReleaseEmergency
        );
        assert!(!g.is_active());

        // Sub-1800 RPM fan under thermal load engages on 2nd consecutive poll
        assert_eq!(
            g.evaluate(Some(80.0), Some(54.0), Some(46.0), Some(60.0), &[1400, 1600]),
            ThermalGuardDecision::NoChange
        );
        assert_eq!(
            g.evaluate(Some(80.0), Some(54.0), Some(46.0), Some(60.0), &[1400, 1600]),
            ThermalGuardDecision::EngageEmergencyMax(
                "Fan < 1800 RPM under thermal load".to_string()
            )
        );
        assert!(g.is_active());

        // Must NOT release while fan is still < 1800 RPM even if temps cooled
        for _ in 0..6 {
            assert_eq!(
                g.evaluate(Some(70.0), Some(50.0), Some(42.0), Some(20.0), &[1500, 2200]),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(g.is_active());
    }
}
