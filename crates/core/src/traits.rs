use crate::error::Result;
use crate::models::{
    BatteryChargeMode, BatteryStatus, CpuBoostMode, DiagnosticReport, DisplayModeInfo, FanMode,
    GpuMode, KeyboardType, PerformanceMode, RgbColor, TelemetrySnapshot,
};

/// Read-only hardware discovery, WMI/ACPI capability probing, and conflict scanning.
pub trait DiagnosticsBackend: Send + Sync {
    fn collect_diagnostics(&self) -> Result<DiagnosticReport>;
}

/// Real-time hardware telemetry provider.
pub trait TelemetryBackend: Send + Sync {
    fn read_telemetry(&self) -> Result<TelemetrySnapshot>;
}

/// Performance mode and cooling fan controller.
pub trait ThermalFanBackend: Send + Sync {
    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<()>;
    fn get_performance_mode(&self) -> Result<PerformanceMode>;
    fn set_fan_mode(&self, mode: FanMode, manual_speed_pct: Option<u8>) -> Result<()>;
    fn keep_alive_tick(&self) -> Result<()>;
}

/// Firmware battery charge-limiting controller.
pub trait BatteryLimitBackend: Send + Sync {
    fn get_battery_status(&self) -> Result<BatteryStatus>;
    fn set_charge_mode(&self, mode: BatteryChargeMode) -> Result<()>;
}

/// Keyboard backlight controller (white backlight, 1-zone RGB, or 4-zone RGB).
pub trait KeyboardBackend: Send + Sync {
    fn detect_keyboard_type(&self) -> Result<KeyboardType>;
    fn set_backlight(&self, enabled: bool, brightness_pct: u8, zones: &[RgbColor]) -> Result<()>;
}

/// Hardware GPU MUX switcher.
pub trait GpuSwitchBackend: Send + Sync {
    fn get_gpu_mode(&self) -> Result<GpuMode>;
    fn set_gpu_mode(&self, mode: GpuMode) -> Result<()>;
}

/// Display refresh rate switcher.
pub trait DisplayBackend: Send + Sync {
    fn get_display_mode(&self) -> Result<DisplayModeInfo>;
    fn set_refresh_rate(&self, hz: u32) -> Result<()>;
}

/// CPU turbo boost, power limits (PL1/PL2), thermal limit (TCC), and voltage offset tuning.
pub trait CpuTuningBackend: Send + Sync {
    fn set_cpu_boost_mode(&self, mode: CpuBoostMode) -> Result<()>;
    fn set_power_limits(&self, pl1_watts: Option<u16>, pl2_watts: Option<u16>) -> Result<()>;
    fn set_tcc_temp_limit(&self, temp_c: u8) -> Result<()>;
    fn set_core_undervolt_mv(&self, offset_mv: i16) -> Result<()>;
}

/// Omen key / global hotkey integration hook.
pub trait HotkeyBackend: Send + Sync {
    fn poll_omen_key_pressed(&self) -> Result<bool>;
}
