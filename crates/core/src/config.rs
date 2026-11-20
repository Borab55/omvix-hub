use crate::error::{OmvixError, Result};
use crate::logging::LogConfig;
use crate::models::{
    BatteryChargeMode, CpuBoostMode, FanCurve, FanMode, GpuMode, PerformanceMode, RgbColor,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub language: String,
    pub minimize_to_tray_on_close: bool,
    pub start_minimized: bool,
    pub start_with_windows: bool,
    pub show_tray_notice_on_close: bool,
    pub theme: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            language: "tr".to_string(),
            minimize_to_tray_on_close: true,
            start_minimized: false,
            start_with_windows: false,
            show_tray_notice_on_close: true,
            theme: "dark".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThermalFanConfig {
    pub performance_mode: PerformanceMode,
    pub fan_mode: FanMode,
    pub manual_speed_pct: u8,
    pub custom_curve: FanCurve,
    pub keep_alive_enabled: bool,
    pub keep_alive_interval_secs: u64,
}

impl Default for ThermalFanConfig {
    fn default() -> Self {
        Self {
            performance_mode: PerformanceMode::Balanced,
            fan_mode: FanMode::Auto,
            manual_speed_pct: 50,
            custom_curve: FanCurve::default(),
            keep_alive_enabled: true,
            keep_alive_interval_secs: 45,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryConfig {
    pub enabled: bool,
    pub mode: BatteryChargeMode,
    pub stop_threshold_pct: u8,
    pub start_threshold_pct: u8,
    pub hold_when_shutdown: bool,
    pub one_time_full_charge: bool,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: false,
            one_time_full_charge: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingConfig {
    pub enabled: bool,
    pub brightness_pct: u8,
    pub zones: [RgbColor; 4],
}

impl Default for LightingConfig {
    fn default() -> Self {
        let default_color = RgbColor::default();
        Self {
            enabled: true,
            brightness_pct: 100,
            zones: [default_color; 4],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoProfileConfig {
    pub enabled: bool,
    pub ac_performance_mode: PerformanceMode,
    pub dc_performance_mode: PerformanceMode,
    pub ac_refresh_hz: Option<u32>,
    pub dc_refresh_hz: Option<u32>,
    pub dim_keyboard_on_battery: bool,
}

impl Default for AutoProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ac_performance_mode: PerformanceMode::Balanced,
            dc_performance_mode: PerformanceMode::Eco,
            ac_refresh_hz: None,
            dc_refresh_hz: Some(60),
            dim_keyboard_on_battery: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OmenKeyAction {
    #[default]
    ToggleWindow,
    CyclePerformanceMode,
    LaunchCustomCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub capture_omen_key: bool,
    pub omen_key_action: OmenKeyAction,
    pub custom_command: String,
    pub global_toggle_shortcut: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            capture_omen_key: true,
            omen_key_action: OmenKeyAction::ToggleWindow,
            custom_command: String::new(),
            global_toggle_shortcut: "Ctrl+Shift+O".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuningConfig {
    pub preferred_gpu_mode: Option<GpuMode>,
    pub cpu_boost_mode: Option<CpuBoostMode>,
    pub pl1_watts: Option<u16>,
    pub pl2_watts: Option<u16>,
    pub tcc_temp_limit_c: Option<u8>,
    pub core_undervolt_mv: Option<i16>,
    pub use_pawn_io_if_available: bool,
}

impl Default for TuningConfig {
    fn default() -> Self {
        Self {
            preferred_gpu_mode: None,
            cpu_boost_mode: None,
            pl1_watts: None,
            pl2_watts: None,
            tcc_temp_limit_c: None,
            core_undervolt_mv: None,
            use_pawn_io_if_available: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub logging: LogConfig,
    pub thermal_fan: ThermalFanConfig,
    pub battery: BatteryConfig,
    pub lighting: LightingConfig,
    pub auto_profile: AutoProfileConfig,
    pub hotkeys: HotkeyConfig,
    pub tuning: TuningConfig,
}

impl AppConfig {
    pub fn default_config_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return PathBuf::from(app_data)
                .join("OmvixHub")
                .join("config.toml");
        }

        if let Some(dirs) = ProjectDirs::from("io", "omvix", "OmvixHub") {
            return dirs.config_dir().join("config.toml");
        }

        PathBuf::from("omvix-config.toml")
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        toml::from_str(&raw).map_err(|e| OmvixError::Config(e.to_string()))
    }

    /// Saves configuration atomically via a temporary file + rename.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized =
            toml::to_string_pretty(self).map_err(|e| OmvixError::Config(e.to_string()))?;
        let tmp_path = path.with_extension("toml.tmp");
        fs::write(&tmp_path, serialized)?;
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_roundtrip() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_cfg_test_{nanos}"));
        let path = dir.join("config.toml");

        let mut cfg = AppConfig::default();
        cfg.general.language = "en".to_string();
        cfg.battery.enabled = true;
        cfg.battery.stop_threshold_pct = 80;
        cfg.battery.start_threshold_pct = 75;
        cfg.logging.max_total_mb = 20;

        cfg.save_to_path(&path).expect("save config");
        let loaded = AppConfig::load_from_path(&path).expect("load config");

        assert_eq!(loaded.general.language, "en");
        assert!(loaded.battery.enabled);
        assert_eq!(loaded.battery.stop_threshold_pct, 80);
        assert_eq!(loaded.battery.start_threshold_pct, 75);
        assert_eq!(loaded.logging.max_total_mb, 20);

        let _ = fs::remove_dir_all(&dir);
    }
}
