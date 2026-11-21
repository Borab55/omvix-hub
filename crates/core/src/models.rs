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
    /// Evaluates target fan speed percentage with linear interpolation and downward hysteresis.
    /// When temperature drops within `hysteresis_c` of `prev_temp_c`, keeps `prev_speed_pct`
    /// to avoid rapid fan oscillation.
    pub fn evaluate(&self, current_temp_c: f32, prev_temp_c: f32, prev_speed_pct: u8) -> u8 {
        if self.points.is_empty() {
            return prev_speed_pct.clamp(0, 100);
        }

        let mut sorted = self.points.clone();
        sorted.sort_by_key(|p| p.temp_c);

        // Downward hysteresis guard: ignore minor cooling jitter.
        if current_temp_c < prev_temp_c && (prev_temp_c - current_temp_c) < self.hysteresis_c as f32
        {
            return prev_speed_pct.clamp(0, 100);
        }

        let first = sorted[0];
        if current_temp_c <= first.temp_c as f32 {
            return first.speed_pct.clamp(0, 100);
        }

        let last = sorted[sorted.len() - 1];
        if current_temp_c >= last.temp_c as f32 {
            return last.speed_pct.clamp(0, 100);
        }

        for pair in sorted.windows(2) {
            let low = pair[0];
            let high = pair[1];
            if current_temp_c >= low.temp_c as f32 && current_temp_c <= high.temp_c as f32 {
                let span = (high.temp_c - low.temp_c) as f32;
                if span <= f32::EPSILON {
                    return high.speed_pct.clamp(0, 100);
                }
                let ratio = (current_temp_c - low.temp_c as f32) / span;
                let interp =
                    low.speed_pct as f32 + ratio * (high.speed_pct as f32 - low.speed_pct as f32);
                return interp.round().clamp(0.0, 100.0) as u8;
            }
        }

        prev_speed_pct.clamp(0, 100)
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
}
