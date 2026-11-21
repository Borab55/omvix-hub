use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemIdentity {
    pub manufacturer: String,
    pub model: String,
    pub system_family: String,
    pub board_id: String,
    pub bios_version: String,
    pub cpu_name: String,
    pub gpu_names: Vec<String>,
    pub os_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThermalProfileScheme {
    OmenV0,
    OmenV1,
    Victus,
    VictusS,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardProfile {
    pub board_id: Cow<'static, str>,
    pub marketing_name: Cow<'static, str>,
    pub thermal_scheme: ThermalProfileScheme,
    pub expected_has_mux: bool,
    pub expected_keyboard: KeyboardType,
    pub expected_charge_limit: bool,
    pub notes: Cow<'static, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PerformanceMode {
    Eco,
    Quiet,
    #[default]
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FanMode {
    #[default]
    Auto,
    Max,
    Manual,
    CustomCurve,
}

pub const FAN_CURVE_SAFETY_FLOOR_TEMP_C: u8 = 80;
pub const FAN_CURVE_SAFETY_FLOOR_SPEED_PCT: u8 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanCurvePoint {
    pub temp_c: u8,
    pub speed_pct: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanCurve {
    pub points: Vec<FanCurvePoint>,
    pub hysteresis_c: u8,
}

impl Default for FanCurve {
    fn default() -> Self {
        Self {
            points: vec![
                FanCurvePoint { temp_c: 35, speed_pct: 25 },
                FanCurvePoint { temp_c: 45, speed_pct: 30 },
                FanCurvePoint { temp_c: 55, speed_pct: 42 },
                FanCurvePoint { temp_c: 65, speed_pct: 58 },
                FanCurvePoint { temp_c: 72, speed_pct: 70 },
                FanCurvePoint { temp_c: 80, speed_pct: 82 },
                FanCurvePoint { temp_c: 88, speed_pct: 92 },
                FanCurvePoint { temp_c: 95, speed_pct: 100 },
            ],
            hysteresis_c: 3,
        }
    }
}

impl FanCurve {
    /// Enforces safe curve invariants: 2..=8 points sorted by temperature, monotonically
    /// non-decreasing fan speed, and a mandatory high-temperature safety floor
    /// (`>= 80°C` -> `>= 50%`). Returns `true` if any point or hysteresis value changed.
    pub fn normalize(&mut self) -> bool {
        let prev_points = self.points.clone();
        let prev_hyst = self.hysteresis_c;
        self.hysteresis_c = self.hysteresis_c.clamp(1, 10);

        if self.points.len() < 2 {
            self.points = Self::default().points;
            return self.points != prev_points || self.hysteresis_c != prev_hyst;
        }

        for pt in &mut self.points {
            pt.temp_c = pt.temp_c.clamp(30, 100);
            pt.speed_pct = pt.speed_pct.clamp(0, 100);
        }
        self.points.sort_by_key(|p| (p.temp_c, p.speed_pct));
        self.points.truncate(8);

        let mut running_max = 0u8;
        for pt in &mut self.points {
            if pt.temp_c >= FAN_CURVE_SAFETY_FLOOR_TEMP_C {
                pt.speed_pct = pt.speed_pct.max(FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);
            }
            if pt.speed_pct < running_max {
                pt.speed_pct = running_max;
            }
            running_max = pt.speed_pct;
        }

        self.points != prev_points || self.hysteresis_c != prev_hyst
    }

    /// Evaluates target fan speed percentage with linear interpolation and downward hysteresis.
    /// When temperature drops within `hysteresis_c` of `prev_temp_c`, keeps `prev_speed_pct`
    /// to avoid rapid fan oscillation. Enforces high-temp safety floor at `>= 80°C`.
    pub fn evaluate(&self, current_temp_c: f32, prev_temp_c: f32, prev_speed_pct: u8) -> u8 {
        let floor_if_hot = |pct: u8| -> u8 {
            if current_temp_c >= FAN_CURVE_SAFETY_FLOOR_TEMP_C as f32 {
                pct.max(FAN_CURVE_SAFETY_FLOOR_SPEED_PCT)
            } else {
                pct
            }
        };

        if self.points.is_empty() {
            return floor_if_hot(prev_speed_pct.clamp(0, 100));
        }

        let mut sorted = self.points.clone();
        sorted.sort_by_key(|p| p.temp_c);

        // Downward hysteresis guard: ignore minor cooling jitter.
        if current_temp_c < prev_temp_c && (prev_temp_c - current_temp_c) < self.hysteresis_c as f32
        {
            return floor_if_hot(prev_speed_pct.clamp(0, 100));
        }

        let first = sorted[0];
        if current_temp_c <= first.temp_c as f32 {
            return floor_if_hot(first.speed_pct.clamp(0, 100));
        }

        let last = sorted[sorted.len() - 1];
        if current_temp_c >= last.temp_c as f32 {
            return floor_if_hot(last.speed_pct.clamp(0, 100));
        }

        for pair in sorted.windows(2) {
            let low = pair[0];
            let high = pair[1];
            if current_temp_c >= low.temp_c as f32 && current_temp_c <= high.temp_c as f32 {
                let span = (high.temp_c - low.temp_c) as f32;
                if span <= f32::EPSILON {
                    return floor_if_hot(high.speed_pct.clamp(0, 100));
                }
                let ratio = (current_temp_c - low.temp_c as f32) / span;
                let interp =
                    low.speed_pct as f32 + ratio * (high.speed_pct as f32 - low.speed_pct as f32);
                return floor_if_hot(interp.round().clamp(0.0, 100.0) as u8);
            }
        }

        floor_if_hot(prev_speed_pct.clamp(0, 100))
    }

    pub fn preset_quiet() -> Self {
        Self {
            points: vec![
                FanCurvePoint { temp_c: 35, speed_pct: 18 },
                FanCurvePoint { temp_c: 45, speed_pct: 22 },
                FanCurvePoint { temp_c: 55, speed_pct: 30 },
                FanCurvePoint { temp_c: 65, speed_pct: 40 },
                FanCurvePoint { temp_c: 72, speed_pct: 48 },
                FanCurvePoint { temp_c: 80, speed_pct: 55 },
                FanCurvePoint { temp_c: 88, speed_pct: 74 },
                FanCurvePoint { temp_c: 95, speed_pct: 92 },
            ],
            hysteresis_c: 4,
        }
    }

    pub fn preset_balanced() -> Self {
        Self::default()
    }

    pub fn preset_aggressive() -> Self {
        Self {
            points: vec![
                FanCurvePoint { temp_c: 35, speed_pct: 35 },
                FanCurvePoint { temp_c: 45, speed_pct: 45 },
                FanCurvePoint { temp_c: 55, speed_pct: 60 },
                FanCurvePoint { temp_c: 65, speed_pct: 75 },
                FanCurvePoint { temp_c: 72, speed_pct: 86 },
                FanCurvePoint { temp_c: 80, speed_pct: 94 },
                FanCurvePoint { temp_c: 88, speed_pct: 100 },
                FanCurvePoint { temp_c: 95, speed_pct: 100 },
            ],
            hysteresis_c: 2,
        }
    }

    pub fn preset_ogh_like() -> Self {
        Self {
            points: vec![
                FanCurvePoint { temp_c: 35, speed_pct: 0 },
                FanCurvePoint { temp_c: 45, speed_pct: 20 },
                FanCurvePoint { temp_c: 55, speed_pct: 32 },
                FanCurvePoint { temp_c: 65, speed_pct: 48 },
                FanCurvePoint { temp_c: 72, speed_pct: 62 },
                FanCurvePoint { temp_c: 80, speed_pct: 76 },
                FanCurvePoint { temp_c: 88, speed_pct: 90 },
                FanCurvePoint { temp_c: 95, speed_pct: 100 },
            ],
            hysteresis_c: 3,
        }
    }

    /// Encodes the fan curve as a compact shareable code (`OMVIXCURVE1|<hyst>|<t:s,...>`).
    pub fn encode_share_code(&self) -> String {
        let mut sorted = self.points.clone();
        sorted.sort_by_key(|p| p.temp_c);
        let pairs: Vec<String> = sorted
            .iter()
            .map(|p| format!("{}:{}", p.temp_c, p.speed_pct))
            .collect();
        format!("OMVIXCURVE1|{}|{}", self.hysteresis_c.clamp(1, 10), pairs.join(","))
    }

    /// Imports a fan curve from either an `OMVIXCURVE1|...` code or JSON, enforcing safety normalization.
    pub fn decode_share_code(raw: &str) -> crate::Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(crate::OmvixError::Config("Fan curve input is empty".to_string()));
        }

        let mut curve = if trimmed.starts_with('{') {
            serde_json::from_str::<Self>(trimmed)
                .map_err(|e| crate::OmvixError::Config(format!("Invalid curve JSON: {e}")))?
        } else {
            let parts: Vec<&str> = trimmed.split('|').collect();
            if parts.len() != 3 || parts[0] != "OMVIXCURVE1" {
                return Err(crate::OmvixError::Config(
                    "Invalid curve format (expected OMVIXCURVE1|<hyst>|<t:s,...> or JSON)".to_string(),
                ));
            }
            let hysteresis_c: u8 = parts[1].trim().parse().map_err(|_| {
                crate::OmvixError::Config("Invalid curve hysteresis value".to_string())
            })?;
            let mut points = Vec::new();
            for pair in parts[2].split(',') {
                let Some((t_str, s_str)) = pair.trim().split_once(':') else {
                    return Err(crate::OmvixError::Config(format!(
                        "Invalid curve point `{pair}` (expected temp:speed)"
                    )));
                };
                let temp_c: u8 = t_str.trim().parse().map_err(|_| {
                    crate::OmvixError::Config(format!("Invalid point temp `{t_str}`"))
                })?;
                let speed_pct: u8 = s_str.trim().parse().map_err(|_| {
                    crate::OmvixError::Config(format!("Invalid point speed `{s_str}`"))
                })?;
                points.push(FanCurvePoint { temp_c, speed_pct });
            }
            if points.len() < 2 {
                return Err(crate::OmvixError::Config(
                    "Imported fan curve must contain at least 2 points".to_string(),
                ));
            }
            Self { points, hysteresis_c }
        };

        if curve.points.len() < 2 {
            return Err(crate::OmvixError::Config(
                "Imported fan curve must contain at least 2 points".to_string(),
            ));
        }
        let _ = curve.normalize();
        Ok(curve)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BatteryChargeMode {
    #[default]
    Auto,
    Inhibit,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BatteryServiceState {
    #[default]
    NotInstalled,
    Running,
    Stopped,
    Starting,
    Stopping,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatteryServiceInfo {
    pub service_name: String,
    pub state: BatteryServiceState,
    pub installed_exe_path: String,
    pub active_binary_path: Option<String>,
    pub data_dir_path: String,
    pub log_file_path: String,
    pub exe_exists: bool,
    pub data_dir_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BatteryStatus {
    pub present: bool,
    pub ac_connected: bool,
    pub charging: bool,
    pub charge_percent: u8,
    pub charge_rate_w: Option<f32>,
    pub voltage_v: Option<f32>,
    pub remaining_capacity_mwh: Option<u32>,
    pub time_remaining_min: Option<u32>,
    pub design_capacity_mwh: Option<u32>,
    pub full_charge_capacity_mwh: Option<u32>,
    pub wear_percent: Option<f32>,
    pub current_mode: BatteryChargeMode,
    pub raw_mode_hex: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KeyboardType {
    None,
    WhiteBacklight,
    SingleZoneRgb,
    FourZoneRgb,
    PerKeyRgb,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for RgbColor {
    fn default() -> Self {
        Self { r: 0, g: 168, b: 255 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GpuMode {
    #[default]
    Hybrid,
    Discrete,
    Optimus,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayModeInfo {
    pub current_hz: u32,
    pub supported_hz: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CpuBoostMode {
    Disabled,
    Enabled,
    #[default]
    Aggressive,
    EfficientAggressive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityStatus {
    pub is_admin: bool,
    pub vbs_status: Option<u32>,
    pub hvci_running: bool,
    pub pawn_io_installed: bool,
    pub undervolt_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictServiceInfo {
    pub service_name: String,
    pub display_name: String,
    pub status: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TelemetrySnapshot {
    pub timestamp: Option<DateTime<Utc>>,
    pub cpu_temp_c: Option<f32>,
    pub chassis_temp_c: Option<f32>,
    pub cpu_usage_pct: Option<f32>,
    pub cpu_clock_mhz: Option<u32>,
    pub cpu_power_w: Option<f32>,
    pub gpu_name: Option<String>,
    pub gpu_sleeping: bool,
    pub gpu_temp_c: Option<f32>,
    pub gpu_usage_pct: Option<f32>,
    pub gpu_power_w: Option<f32>,
    pub gpu_power_limit_w: Option<f32>,
    pub gpu_clock_mhz: Option<u32>,
    pub gpu_mem_clock_mhz: Option<u32>,
    pub gpu_mem_used_mb: Option<u32>,
    pub gpu_mem_total_mb: Option<u32>,
    pub gpu_pstate: Option<String>,
    pub gpu_throttle_reason: Option<String>,
    pub ram_used_gb: Option<f32>,
    pub ram_total_gb: Option<f32>,
    pub fan_rpms: Vec<u32>,
    pub fan_max_rpms: Vec<u32>,
    pub ac_connected: Option<bool>,
    pub battery_pct: Option<u8>,
    pub battery_rate_w: Option<f32>,
    pub battery_voltage_v: Option<f32>,
    pub battery_remaining_mwh: Option<u32>,
    pub battery_time_remaining_min: Option<u32>,
    pub disk_read_mb_s: Option<f32>,
    pub disk_write_mb_s: Option<f32>,
    pub net_rx_mb_s: Option<f32>,
    pub net_tx_mb_s: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmiProbeEntry {
    pub name: String,
    pub command: u32,
    pub command_type: u32,
    pub output_size: u32,
    pub success: bool,
    pub return_code: Option<u32>,
    pub data_hex: String,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub generated_at: DateTime<Utc>,
    pub app_version: String,
    pub identity: SystemIdentity,
    pub matched_board: Option<BoardProfile>,
    pub security: SecurityStatus,
    pub wmi_hpqbintm_available: bool,
    pub wmi_hpqbevnt_available: bool,
    pub wmi_error: Option<String>,
    pub conflict_services: Vec<ConflictServiceInfo>,
    pub probes: Vec<WmiProbeEntry>,
}

/// Installation & autostart integrity status for the protected `Program Files` GUI copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledCopyStatus {
    pub installed_path: String,
    pub installed_exists: bool,
    pub running_from_installed: bool,
    pub installed_is_outdated: bool,
    pub task_exists: bool,
    pub task_path_matches: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_curve_interpolates_and_applies_hysteresis() {
        let curve = FanCurve {
            points: vec![
                FanCurvePoint {
                    temp_c: 40,
                    speed_pct: 20,
                },
                FanCurvePoint {
                    temp_c: 80,
                    speed_pct: 100,
                },
            ],
            hysteresis_c: 4,
        };

        // Clamped below minimum
        assert_eq!(curve.evaluate(30.0, 30.0, 20), 20);
        // Exact midpoint -> 60%
        assert_eq!(curve.evaluate(60.0, 50.0, 40), 60);
        // Minor drop of 2C (< hysteresis 4C) -> stays at 60%
        assert_eq!(curve.evaluate(58.0, 60.0, 60), 60);
        // Drop of 5C (>= hysteresis 4C) -> interpolates down to 50%
        assert_eq!(curve.evaluate(55.0, 60.0, 60), 50);
        // Clamped above maximum
        assert_eq!(curve.evaluate(95.0, 80.0, 100), 100);
    }

    #[test]
    fn fan_curve_normalize_enforces_monotonic_and_high_temp_floor() {
        let mut curve = FanCurve {
            points: vec![
                FanCurvePoint { temp_c: 40, speed_pct: 40 },
                FanCurvePoint { temp_c: 60, speed_pct: 25 },
                FanCurvePoint { temp_c: 85, speed_pct: 10 },
                FanCurvePoint { temp_c: 92, speed_pct: 45 },
            ],
            hysteresis_c: 20,
        };
        assert!(curve.normalize());
        assert_eq!(curve.hysteresis_c, 10);
        assert_eq!(curve.points[0].speed_pct, 40);
        assert_eq!(curve.points[1].speed_pct, 40); // monotonically non-decreasing
        assert_eq!(curve.points[2].speed_pct, FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);
        assert_eq!(curve.points[3].speed_pct, FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);

        // Second normalize is a no-op
        assert!(!curve.normalize());

        // Even if all points sit below 80°C, evaluating >= 80°C applies the 50% safety floor.
        let sub80 = FanCurve {
            points: vec![
                FanCurvePoint { temp_c: 40, speed_pct: 10 },
                FanCurvePoint { temp_c: 75, speed_pct: 20 },
            ],
            hysteresis_c: 3,
        };
        assert_eq!(sub80.evaluate(82.0, 75.0, 20), FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);
    }

    #[test]
    fn fan_curve_presets_are_valid_monotonic_and_respect_safety_floor() {
        for mut preset in [
            FanCurve::preset_quiet(),
            FanCurve::preset_balanced(),
            FanCurve::preset_aggressive(),
            FanCurve::preset_ogh_like(),
        ] {
            let before = preset.clone();
            assert!(!preset.normalize());
            assert_eq!(preset, before);
            assert_eq!(preset.points.len(), 8);
            assert!(preset.evaluate(80.0, 72.0, 40) >= FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);
        }
    }

    #[test]
    fn fan_curve_share_code_and_json_roundtrip_enforces_safety_normalization() {
        let orig = FanCurve::preset_ogh_like();
        let code = orig.encode_share_code();
        assert!(code.starts_with("OMVIXCURVE1|3|"));
        let decoded = FanCurve::decode_share_code(&code).expect("decode code");
        assert_eq!(decoded, orig);

        let json = serde_json::to_string(&orig).expect("json");
        let decoded_json = FanCurve::decode_share_code(&json).expect("decode json");
        assert_eq!(decoded_json, orig);

        assert!(FanCurve::decode_share_code("").is_err());
        assert!(FanCurve::decode_share_code("INVALID|3|40:20,80:80").is_err());

        // Importing an unsafe high-temp curve is normalized up to >= 50% at >= 80C
        let unsafe_curve = FanCurve::decode_share_code("OMVIXCURVE1|3|40:30,90:10").expect("import unsafe");
        assert_eq!(unsafe_curve.points[1].speed_pct, FAN_CURVE_SAFETY_FLOOR_SPEED_PCT);
    }
}
