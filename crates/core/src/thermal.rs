use crate::boards::lookup_board;
use crate::models::{FanMode, PerformanceMode, ThermalProfileScheme};

/// Default maximum WMI fan level step when firmware `FanMaxGet` (0x26) is unavailable.
pub const DEFAULT_MAX_FAN_LEVEL: u8 = 55;

/// Minimum safe non-zero WMI fan level (~1800 RPM) enforced when temperatures are elevated.
pub const MIN_SAFE_FAN_LEVEL: u8 = 18;

/// Minimum fan RPM expected under warm load before the thermal guard overrides to Max Fan.
pub const MIN_GUARD_FAN_RPM: u32 = 1800;

/// Seconds without any temperature sensor before forcing BIOS `Auto` fan fallback (R1).
pub const DEFAULT_SENSOR_LOSS_TIMEOUT_SECS: u64 = 6;

/// Configurable thermal guard bounds (R7).
pub const MIN_GUARD_CPU_TEMP_C: u8 = 90;
pub const MAX_GUARD_CPU_TEMP_C: u8 = 98;
pub const DEFAULT_GUARD_CPU_TEMP_C: u8 = 95;
pub const MIN_GUARD_SUSTAIN_SECS: u8 = 10;
pub const MAX_GUARD_SUSTAIN_SECS: u8 = 30;
pub const DEFAULT_GUARD_SUSTAIN_SECS: u8 = 10;

/// Emergency Max Fan retry & RPM verification constants (R3).
pub const GUARD_MAX_RETRIES: u8 = 3;
pub const GUARD_RETRY_INTERVAL_SECS: f64 = 2.0;
pub const GUARD_RPM_VERIFY_DELAY_SECS: f64 = 4.0;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorLossDecision {
    NoChange,
    EngageAutoFallback,
    RestoreUserMode,
}

/// Triggers safe BIOS `Auto` fan fallback when all temperature telemetry disappears for
/// `timeout_secs` seconds, then restores user fan control once any thermal sensor returns (R1).
#[derive(Debug, Clone)]
pub struct SensorLossGuard {
    timeout_secs: u64,
    missing_since_secs: Option<f64>,
    active: bool,
}

impl Default for SensorLossGuard {
    fn default() -> Self {
        Self::new(DEFAULT_SENSOR_LOSS_TIMEOUT_SECS)
    }
}

impl SensorLossGuard {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            timeout_secs: timeout_secs.max(1),
            missing_since_secs: None,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.missing_since_secs = None;
        self.active = false;
    }

    pub fn evaluate(&mut self, has_any_temp: bool, now_secs: f64) -> SensorLossDecision {
        if has_any_temp {
            self.missing_since_secs = None;
            if self.active {
                self.active = false;
                return SensorLossDecision::RestoreUserMode;
            }
            return SensorLossDecision::NoChange;
        }

        if self.active {
            return SensorLossDecision::NoChange;
        }

        let start = *self.missing_since_secs.get_or_insert(now_secs);
        if (now_secs - start) >= self.timeout_secs as f64 {
            self.active = true;
            return SensorLossDecision::EngageAutoFallback;
        }

        SensorLossDecision::NoChange
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThermalGuardConfig {
    pub cpu_threshold_c: u8,
    pub sustain_secs: u8,
}

impl Default for ThermalGuardConfig {
    fn default() -> Self {
        Self {
            cpu_threshold_c: DEFAULT_GUARD_CPU_TEMP_C,
            sustain_secs: DEFAULT_GUARD_SUSTAIN_SECS,
        }
    }
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
    cpu_hot_since_secs: Option<f64>,
    gpu_hot_since_secs: Option<f64>,
    chassis_hot_since_secs: Option<f64>,
    low_rpm_since_secs: Option<f64>,
    cooled_since_secs: Option<f64>,
}

fn track_sustained(slot: &mut Option<f64>, cond: bool, now_secs: f64, req_secs: f64) -> bool {
    if cond {
        let start = *slot.get_or_insert(now_secs);
        (now_secs - start) >= req_secs
    } else {
        *slot = None;
        false
    }
}

impl ThermalSafetyGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        cfg: ThermalGuardConfig,
        fan_mode: FanMode,
        cpu_c: Option<f32>,
        gpu_c: Option<f32>,
        chassis_c: Option<f32>,
        cpu_power_w: Option<f32>,
        fan_rpms: &[u32],
        now_secs: f64,
    ) -> ThermalGuardDecision {
        let cpu_thresh = cfg
            .cpu_threshold_c
            .clamp(MIN_GUARD_CPU_TEMP_C, MAX_GUARD_CPU_TEMP_C) as f32;
        let sustain = cfg
            .sustain_secs
            .clamp(MIN_GUARD_SUSTAIN_SECS, MAX_GUARD_SUSTAIN_SECS) as f64;

        let cpu_sustained = track_sustained(
            &mut self.cpu_hot_since_secs,
            cpu_c.is_some_and(|t| t >= cpu_thresh),
            now_secs,
            sustain,
        );
        let gpu_sustained = track_sustained(
            &mut self.gpu_hot_since_secs,
            gpu_c.is_some_and(|t| t >= 86.0),
            now_secs,
            5.0,
        );
        let chassis_sustained = track_sustained(
            &mut self.chassis_hot_since_secs,
            chassis_c.is_some_and(|t| t >= 54.0),
            now_secs,
            5.0,
        );

        let (low_rpm_cond, low_rpm_req_secs) = if fan_mode == FanMode::Auto {
            let all_fans_low =
                !fan_rpms.is_empty() && fan_rpms.iter().all(|&r| r < MIN_GUARD_FAN_RPM);
            let heavy_thermal_load = cpu_c.is_some_and(|t| t >= 85.0)
                || gpu_c.is_some_and(|t| t >= 80.0)
                || chassis_c.is_some_and(|t| t >= 50.0)
                || (cpu_c.is_none() && cpu_power_w.is_some_and(|w| w >= 70.0));
            (all_fans_low && heavy_thermal_load, 5.0)
        } else {
            let any_fan_low =
                !fan_rpms.is_empty() && fan_rpms.iter().any(|&r| r < MIN_GUARD_FAN_RPM);
            let warm_under_load = cpu_c.is_some_and(|t| t >= 78.0)
                || gpu_c.is_some_and(|t| t >= 74.0)
                || chassis_c.is_some_and(|t| t >= 48.0)
                || (cpu_c.is_none() && cpu_power_w.is_some_and(|w| w >= 55.0));
            (any_fan_low && warm_under_load, 3.0)
        };

        let low_rpm_sustained = track_sustained(
            &mut self.low_rpm_since_secs,
            low_rpm_cond,
            now_secs,
            low_rpm_req_secs,
        );

        let trigger_reason = if cpu_sustained {
            Some(format!(
                "CPU >= {cpu_thresh:.0}°C for {}s",
                sustain.round() as u32
            ))
        } else if gpu_sustained {
            Some("GPU >= 86°C for 5s".to_string())
        } else if chassis_sustained {
            Some("Chassis >= 54°C for 5s".to_string())
        } else if low_rpm_sustained {
            Some("Fan < 1800 RPM under thermal load".to_string())
        } else {
            None
        };

        if let Some(reason) = trigger_reason {
            self.cooled_since_secs = None;
            if !self.active {
                self.active = true;
                return ThermalGuardDecision::EngageEmergencyMax(reason);
            }
            return ThermalGuardDecision::NoChange;
        }

        if self.active {
            let release_cpu_max = (cpu_thresh - 12.0).min(78.0);
            let fans_low_any =
                !fan_rpms.is_empty() && fan_rpms.iter().any(|&r| r < MIN_GUARD_FAN_RPM);
            let has_any_temp = cpu_c.is_some() || gpu_c.is_some() || chassis_c.is_some();
            let cooled = has_any_temp
                && cpu_c.is_none_or(|t| t <= release_cpu_max)
                && gpu_c.is_none_or(|t| t <= 74.0)
                && chassis_c.is_none_or(|t| t <= 46.0)
                && (fan_mode == FanMode::Auto || !fans_low_any);

            if track_sustained(&mut self.cooled_since_secs, cooled, now_secs, 5.0) {
                self.active = false;
                self.cooled_since_secs = None;
                return ThermalGuardDecision::ReleaseEmergency;
            }
        }

        ThermalGuardDecision::NoChange
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardMaxAction {
    Wait,
    RetryMax { attempt: u8 },
    Verified,
    FallbackToAuto(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GuardRetryPhase {
    Idle,
    PendingRetry { next_attempt_at_secs: f64 },
    VerifyingRpm { verify_deadline_secs: f64 },
    Verified,
    FailedFallbackAuto,
}

/// Retries failed `set_fan_mode(Max)` calls up to 3 attempts at 2s intervals, verifies
/// physical RPM spin-up within 4s, and falls back to BIOS `Auto` if Max cannot engage (R3).
#[derive(Debug, Clone)]
pub struct GuardMaxRetryController {
    phase: GuardRetryPhase,
    attempts_done: u8,
    baseline_max_rpm: u32,
}

impl Default for GuardMaxRetryController {
    fn default() -> Self {
        Self::new()
    }
}

impl GuardMaxRetryController {
    pub fn new() -> Self {
        Self {
            phase: GuardRetryPhase::Idle,
            attempts_done: 0,
            baseline_max_rpm: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn is_fallback_auto(&self) -> bool {
        self.phase == GuardRetryPhase::FailedFallbackAuto
    }

    pub fn begin_engage(&mut self, baseline_rpms: &[u32], _now_secs: f64) {
        self.attempts_done = 0;
        self.baseline_max_rpm = baseline_rpms.iter().copied().max().unwrap_or(0);
        self.phase = GuardRetryPhase::Idle;
    }

    pub fn record_attempt_result(&mut self, ok: bool, now_secs: f64) -> GuardMaxAction {
        self.attempts_done = self.attempts_done.saturating_add(1);
        if ok {
            self.phase = GuardRetryPhase::VerifyingRpm {
                verify_deadline_secs: now_secs + GUARD_RPM_VERIFY_DELAY_SECS,
            };
            GuardMaxAction::Wait
        } else if self.attempts_done < GUARD_MAX_RETRIES {
            self.phase = GuardRetryPhase::PendingRetry {
                next_attempt_at_secs: now_secs + GUARD_RETRY_INTERVAL_SECS,
            };
            GuardMaxAction::Wait
        } else {
            self.phase = GuardRetryPhase::FailedFallbackAuto;
            GuardMaxAction::FallbackToAuto(format!(
                "Emergency Max Fan failed after {GUARD_MAX_RETRIES} attempts; restored BIOS Auto"
            ))
        }
    }

    pub fn tick(
        &mut self,
        current_rpms: &[u32],
        fan_max_rpms: &[u32],
        now_secs: f64,
    ) -> GuardMaxAction {
        match self.phase {
            GuardRetryPhase::PendingRetry {
                next_attempt_at_secs,
            } => {
                if now_secs >= next_attempt_at_secs {
                    GuardMaxAction::RetryMax {
                        attempt: self.attempts_done + 1,
                    }
                } else {
                    GuardMaxAction::Wait
                }
            }
            GuardRetryPhase::VerifyingRpm {
                verify_deadline_secs,
            } => {
                let cur_max = current_rpms.iter().copied().max().unwrap_or(0);
                let target_floor = fan_max_rpms
                    .iter()
                    .copied()
                    .max()
                    .map(|m| m.saturating_mul(55) / 100)
                    .unwrap_or(2800)
                    .max(2400);

                if !current_rpms.is_empty()
                    && cur_max >= MIN_GUARD_FAN_RPM
                    && (cur_max >= self.baseline_max_rpm.saturating_add(400)
                        || cur_max >= target_floor)
                {
                    self.phase = GuardRetryPhase::Verified;
                    return GuardMaxAction::Verified;
                }

                if now_secs >= verify_deadline_secs {
                    if self.attempts_done < GUARD_MAX_RETRIES {
                        return GuardMaxAction::RetryMax {
                            attempt: self.attempts_done + 1,
                        };
                    }
                    self.phase = GuardRetryPhase::FailedFallbackAuto;
                    return GuardMaxAction::FallbackToAuto(
                        "Emergency Max Fan did not spin up fans; restored BIOS Auto".to_string(),
                    );
                }

                GuardMaxAction::Wait
            }
            GuardRetryPhase::Idle
            | GuardRetryPhase::Verified
            | GuardRetryPhase::FailedFallbackAuto => GuardMaxAction::Wait,
        }
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
        assert!(is_system_cool_for_zero_rpm(
            Some(50.0),
            Some(45.0),
            Some(38.0)
        ));
        assert!(is_system_cool_for_zero_rpm(None, Some(52.0), None));
        assert!(!is_system_cool_for_zero_rpm(None, None, None));
        assert!(!is_system_cool_for_zero_rpm(
            Some(65.0),
            Some(45.0),
            Some(38.0)
        ));
        assert!(!is_system_cool_for_zero_rpm(
            Some(55.0),
            Some(60.0),
            Some(38.0)
        ));
        assert!(!is_system_cool_for_zero_rpm(
            Some(55.0),
            Some(45.0),
            Some(46.0)
        ));

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
        let cfg = ThermalGuardConfig {
            cpu_threshold_c: 95,
            sustain_secs: 10,
        };

        // Cool idle or momentary CPU watt spikes on Auto must NOT trip the guard
        for tick in 0..8 {
            let t = tick as f64;
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::Auto,
                    Some(55.0),
                    Some(44.0),
                    Some(39.0),
                    Some(75.0),
                    &[1400, 0],
                    t,
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(!g.is_active());

        // Short 2s startup spike at 91-96°C resets when temp drops -> NO emergency override!
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::Auto,
                Some(96.0),
                Some(45.0),
                Some(40.0),
                Some(60.0),
                &[2200, 0],
                10.0
            ),
            ThermalGuardDecision::NoChange
        );
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::Auto,
                Some(96.0),
                Some(45.0),
                Some(40.0),
                Some(60.0),
                &[2400, 0],
                11.5
            ),
            ThermalGuardDecision::NoChange
        );
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::Auto,
                Some(68.0),
                Some(45.0),
                Some(40.0),
                Some(25.0),
                &[2400, 0],
                12.5
            ),
            ThermalGuardDecision::NoChange
        );
        assert!(!g.is_active());

        // Sustained >= 95°C for >= 10s engages emergency Max
        for step in 0..10 {
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::CustomCurve,
                    Some(96.0),
                    Some(62.0),
                    Some(47.0),
                    Some(70.0),
                    &[3200, 3200],
                    20.0 + step as f64,
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::CustomCurve,
                Some(96.0),
                Some(62.0),
                Some(47.0),
                Some(70.0),
                &[3200, 3200],
                30.0,
            ),
            ThermalGuardDecision::EngageEmergencyMax("CPU >= 95°C for 10s".to_string())
        );
        assert!(g.is_active());

        // Still warm (83°C) -> no release even after 6s
        for step in 1..=6 {
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::CustomCurve,
                    Some(83.0),
                    Some(60.0),
                    Some(45.0),
                    Some(30.0),
                    &[5800, 5800],
                    30.0 + step as f64,
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(g.is_active());

        // Cool <= 78C and fans >= 1800 RPM -> releases after 5 consecutive cool seconds
        for step in 0..5 {
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::CustomCurve,
                    Some(72.0),
                    Some(55.0),
                    Some(43.0),
                    Some(20.0),
                    &[5800, 5800],
                    40.0 + step as f64,
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::CustomCurve,
                Some(72.0),
                Some(55.0),
                Some(43.0),
                Some(20.0),
                &[5800, 5800],
                45.0,
            ),
            ThermalGuardDecision::ReleaseEmergency
        );
        assert!(!g.is_active());

        // Sub-1800 RPM fan under thermal load on CustomCurve engages after >= 3s
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::CustomCurve,
                Some(80.0),
                Some(54.0),
                Some(46.0),
                Some(60.0),
                &[1400, 1600],
                50.0
            ),
            ThermalGuardDecision::NoChange
        );
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::CustomCurve,
                Some(80.0),
                Some(54.0),
                Some(46.0),
                Some(60.0),
                &[1400, 1600],
                52.0
            ),
            ThermalGuardDecision::NoChange
        );
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::CustomCurve,
                Some(80.0),
                Some(54.0),
                Some(46.0),
                Some(60.0),
                &[1400, 1600],
                53.0
            ),
            ThermalGuardDecision::EngageEmergencyMax(
                "Fan < 1800 RPM under thermal load".to_string()
            )
        );
        assert!(g.is_active());

        // Must NOT release on CustomCurve while a fan is still < 1800 RPM even if cooled
        for step in 0..7 {
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::CustomCurve,
                    Some(70.0),
                    Some(50.0),
                    Some(42.0),
                    Some(20.0),
                    &[1500, 2200],
                    60.0 + step as f64,
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(g.is_active());
    }

    #[test]
    fn sensor_loss_guard_engages_after_timeout_and_restores_on_recovery() {
        let mut sl = SensorLossGuard::new(6);
        assert_eq!(sl.evaluate(true, 0.0), SensorLossDecision::NoChange);
        assert_eq!(sl.evaluate(false, 1.0), SensorLossDecision::NoChange);
        assert_eq!(sl.evaluate(false, 6.9), SensorLossDecision::NoChange);
        assert!(!sl.is_active());

        assert_eq!(
            sl.evaluate(false, 7.0),
            SensorLossDecision::EngageAutoFallback
        );
        assert!(sl.is_active());
        assert_eq!(sl.evaluate(false, 12.0), SensorLossDecision::NoChange);

        assert_eq!(sl.evaluate(true, 13.0), SensorLossDecision::RestoreUserMode);
        assert!(!sl.is_active());
    }

    #[test]
    fn guard_max_retry_controller_retries_and_verifies_or_falls_back_to_auto() {
        // Path 1: 3 failed attempts at 2s intervals -> FallbackToAuto
        let mut rc = GuardMaxRetryController::new();
        rc.begin_engage(&[1500, 1500], 0.0);
        assert_eq!(rc.record_attempt_result(false, 0.0), GuardMaxAction::Wait);
        assert_eq!(
            rc.tick(&[1500, 1500], &[5800, 6100], 1.5),
            GuardMaxAction::Wait
        );
        assert_eq!(
            rc.tick(&[1500, 1500], &[5800, 6100], 2.0),
            GuardMaxAction::RetryMax { attempt: 2 }
        );
        assert_eq!(rc.record_attempt_result(false, 2.0), GuardMaxAction::Wait);
        assert_eq!(
            rc.tick(&[1500, 1500], &[5800, 6100], 4.0),
            GuardMaxAction::RetryMax { attempt: 3 }
        );
        assert!(matches!(
            rc.record_attempt_result(false, 4.0),
            GuardMaxAction::FallbackToAuto(_)
        ));
        assert!(rc.is_fallback_auto());

        // Path 2: Command succeeds and RPM rises -> Verified
        let mut rc2 = GuardMaxRetryController::new();
        rc2.begin_engage(&[1800, 1800], 10.0);
        assert_eq!(rc2.record_attempt_result(true, 10.0), GuardMaxAction::Wait);
        assert_eq!(
            rc2.tick(&[1900, 1900], &[5800, 6100], 11.0),
            GuardMaxAction::Wait
        );
        assert_eq!(
            rc2.tick(&[3800, 4100], &[5800, 6100], 12.0),
            GuardMaxAction::Verified
        );

        // Path 3: Command succeeds but RPM stays stuck for 4s -> retries then falls back
        let mut rc3 = GuardMaxRetryController::new();
        rc3.begin_engage(&[1200, 1200], 0.0);
        assert_eq!(rc3.record_attempt_result(true, 0.0), GuardMaxAction::Wait);
        assert_eq!(
            rc3.tick(&[1200, 1200], &[5800, 6100], 4.0),
            GuardMaxAction::RetryMax { attempt: 2 }
        );

        // Path 4: Small spin below MIN_GUARD_FAN_RPM (0 -> 400 RPM) must NOT falsely verify
        let mut rc4 = GuardMaxRetryController::new();
        rc4.begin_engage(&[0, 0], 0.0);
        assert_eq!(rc4.record_attempt_result(true, 0.0), GuardMaxAction::Wait);
        assert_eq!(
            rc4.tick(&[400, 400], &[5800, 6100], 1.0),
            GuardMaxAction::Wait
        );
        assert_eq!(
            rc4.tick(&[400, 400], &[5800, 6100], 4.0),
            GuardMaxAction::RetryMax { attempt: 2 }
        );
    }

    #[test]
    fn thermal_safety_guard_does_not_release_on_total_sensor_loss() {
        let mut g = ThermalSafetyGuard::new();
        let cfg = ThermalGuardConfig::default();

        // Trigger sustained thermal guard
        assert_eq!(
            g.evaluate(
                cfg,
                FanMode::Manual,
                Some(96.0),
                Some(60.0),
                Some(40.0),
                None,
                &[3000, 3000],
                0.0
            ),
            ThermalGuardDecision::NoChange
        );
        let engage_t = (cfg.sustain_secs as f64).max(15.0) + 1.0;
        let engaged = g.evaluate(
            cfg,
            FanMode::Manual,
            Some(96.0),
            Some(60.0),
            Some(40.0),
            None,
            &[3000, 3000],
            engage_t,
        );
        assert!(matches!(
            engaged,
            ThermalGuardDecision::EngageEmergencyMax(_)
        ));
        assert!(g.is_active());

        // Total sensor loss (all temps None) must NOT be misclassified as cooled
        for dt in [1.0, 6.0, 15.0] {
            assert_eq!(
                g.evaluate(
                    cfg,
                    FanMode::Max,
                    None,
                    None,
                    None,
                    None,
                    &[5800, 5800],
                    engage_t + dt
                ),
                ThermalGuardDecision::NoChange
            );
        }
        assert!(g.is_active());
    }
}
