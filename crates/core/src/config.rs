use crate::app_profiles::{normalize_exe_name, AppProfilesConfig};
use crate::error::{OmvixError, Result};
use crate::logging::LogConfig;
use crate::models::{
    BatteryChargeMode, CpuBoostMode, FanCurve, FanCurvePreset, FanMode, GpuMode, PerformanceMode,
    RgbColor,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::thermal::{DEFAULT_GUARD_CPU_TEMP_C, DEFAULT_GUARD_SUSTAIN_SECS};

pub const CURRENT_CONFIG_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomThemeConfig {
    pub name: String,
    pub dark_base: bool,
    pub accent: [u8; 3],
    pub canvas_bg: [u8; 3],
    pub sidebar_bg: [u8; 3],
    pub card_bg: [u8; 3],
    pub card_border: [u8; 3],
    pub text_primary: [u8; 3],
}

impl Default for CustomThemeConfig {
    fn default() -> Self {
        Self::preset_dark_slate()
    }
}

impl CustomThemeConfig {
    pub fn preset_dark_slate() -> Self {
        Self {
            name: "Dark Graphite".to_string(),
            dark_base: true,
            accent: [0, 180, 216],
            canvas_bg: [22, 22, 24],
            sidebar_bg: [18, 18, 20],
            card_bg: [31, 31, 34],
            card_border: [52, 52, 57],
            text_primary: [238, 238, 240],
        }
    }

    /// Detects the pre-v0.3 bluish "Dark Slate" default palette for seamless migration
    /// to neutral charcoal without overwriting user-customized themes.
    pub fn is_legacy_bluish_dark_slate(&self) -> bool {
        self.canvas_bg == [15, 20, 27]
            && self.card_bg == [22, 29, 39]
            && self.card_border == [38, 49, 64]
    }

    pub fn preset_clean_light() -> Self {
        Self {
            name: "Clean Light".to_string(),
            dark_base: false,
            accent: [2, 132, 199],
            canvas_bg: [244, 246, 249],
            sidebar_bg: [235, 240, 246],
            card_bg: [255, 255, 255],
            card_border: [216, 223, 232],
            text_primary: [24, 32, 44],
        }
    }

    pub fn preset_omen_crimson() -> Self {
        Self {
            name: "OMEN Crimson".to_string(),
            dark_base: true,
            accent: [230, 57, 70],
            canvas_bg: [16, 14, 18],
            sidebar_bg: [12, 10, 14],
            card_bg: [26, 21, 28],
            card_border: [58, 36, 46],
            text_primary: [244, 238, 242],
        }
    }

    pub fn preset_emerald_matrix() -> Self {
        Self {
            name: "Emerald Matrix".to_string(),
            dark_base: true,
            accent: [16, 185, 129],
            canvas_bg: [11, 18, 16],
            sidebar_bg: [9, 14, 13],
            card_bg: [18, 29, 26],
            card_border: [31, 56, 48],
            text_primary: [228, 244, 238],
        }
    }

    pub fn preset_midnight_violet() -> Self {
        Self {
            name: "Midnight Violet".to_string(),
            dark_base: true,
            accent: [139, 92, 246],
            canvas_bg: [15, 14, 24],
            sidebar_bg: [12, 11, 20],
            card_bg: [24, 22, 38],
            card_border: [48, 42, 74],
            text_primary: [238, 236, 248],
        }
    }

    pub fn preset_high_contrast() -> Self {
        Self {
            name: "High Contrast".to_string(),
            dark_base: true,
            accent: [255, 224, 51],
            canvas_bg: [0, 0, 0],
            sidebar_bg: [0, 0, 0],
            card_bg: [12, 12, 14],
            card_border: [220, 220, 225],
            text_primary: [255, 255, 255],
        }
    }

    pub fn is_high_contrast(&self) -> bool {
        self.canvas_bg == [0, 0, 0] && self.text_primary == [255, 255, 255]
    }

    pub fn presets() -> Vec<Self> {
        vec![
            Self::preset_dark_slate(),
            Self::preset_clean_light(),
            Self::preset_omen_crimson(),
            Self::preset_emerald_matrix(),
            Self::preset_midnight_violet(),
            Self::preset_high_contrast(),
        ]
    }

    /// Encodes this theme into a compact shareable ASCII code (`OMVIX1|...`).
    pub fn encode_share_code(&self) -> String {
        let mode = if self.dark_base { "D" } else { "L" };
        let clean_name = self.name.replace(['|', '\r', '\n'], " ").trim().to_string();
        let display_name = if clean_name.is_empty() {
            "Custom".to_string()
        } else {
            clean_name
        };
        format!(
            "OMVIX1|{}|{}|{}|{}|{}|{}|{}|{}",
            mode,
            rgb_to_hex(self.accent),
            rgb_to_hex(self.canvas_bg),
            rgb_to_hex(self.sidebar_bg),
            rgb_to_hex(self.card_bg),
            rgb_to_hex(self.card_border),
            rgb_to_hex(self.text_primary),
            display_name
        )
    }

    /// Decodes either an `OMVIX1|...` compact share code or a raw JSON `CustomThemeConfig`.
    pub fn decode_share_code(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(OmvixError::Config("Theme share code is empty".to_string()));
        }

        if trimmed.starts_with('{') {
            return serde_json::from_str::<Self>(trimmed)
                .map_err(|e| OmvixError::Config(format!("Invalid theme JSON: {e}")));
        }

        let parts: Vec<&str> = trimmed.split('|').collect();
        if parts.len() < 8 || parts[0] != "OMVIX1" {
            return Err(OmvixError::Config(
                "Invalid theme code format (expected OMVIX1|... or JSON)".to_string(),
            ));
        }

        let dark_base = match parts[1] {
            "D" | "d" | "dark" | "1" | "true" => true,
            "L" | "l" | "light" | "0" | "false" => false,
            other => {
                return Err(OmvixError::Config(format!(
                    "Invalid theme base flag `{other}` (expected D or L)"
                )));
            }
        };

        let accent = hex_to_rgb(parts[2])?;
        let canvas_bg = hex_to_rgb(parts[3])?;
        let sidebar_bg = hex_to_rgb(parts[4])?;
        let card_bg = hex_to_rgb(parts[5])?;
        let card_border = hex_to_rgb(parts[6])?;
        let text_primary = hex_to_rgb(parts[7])?;
        let name = parts
            .get(8)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("Imported Theme")
            .to_string();

        Ok(Self {
            name,
            dark_base,
            accent,
            canvas_bg,
            sidebar_bg,
            card_bg,
            card_border,
            text_primary,
        })
    }
}

fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn hex_to_rgb(s: &str) -> Result<[u8; 3]> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return Err(OmvixError::Config(format!(
            "Invalid RGB hex color `{s}` (expected 6 hex chars)"
        )));
    }
    let r = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|e| OmvixError::Config(format!("Invalid red hex in `{s}`: {e}")))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|e| OmvixError::Config(format!("Invalid green hex in `{s}`: {e}")))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|e| OmvixError::Config(format!("Invalid blue hex in `{s}`: {e}")))?;
    Ok([r, g, b])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryProfile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub stop_threshold_pct: u8,
    pub start_threshold_pct: u8,
    pub mode: BatteryChargeMode,
    pub builtin: bool,
}

impl Default for BatteryProfile {
    fn default() -> Self {
        Self {
            id: "balanced".to_string(),
            name: "Balanced".to_string(),
            enabled: true,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            mode: BatteryChargeMode::Inhibit,
            builtin: false,
        }
    }
}

impl BatteryProfile {
    pub fn builtins() -> Vec<Self> {
        vec![
            Self {
                id: "desktop".to_string(),
                name: "Desktop (60%)".to_string(),
                enabled: true,
                stop_threshold_pct: 60,
                start_threshold_pct: 55,
                mode: BatteryChargeMode::Inhibit,
                builtin: true,
            },
            Self {
                id: "balanced".to_string(),
                name: "Balanced (80%)".to_string(),
                enabled: true,
                stop_threshold_pct: 80,
                start_threshold_pct: 75,
                mode: BatteryChargeMode::Inhibit,
                builtin: true,
            },
            Self {
                id: "travel".to_string(),
                name: "Travel (90%)".to_string(),
                enabled: true,
                stop_threshold_pct: 90,
                start_threshold_pct: 85,
                mode: BatteryChargeMode::Inhibit,
                builtin: true,
            },
            Self {
                id: "full".to_string(),
                name: "Full Charge (100%)".to_string(),
                enabled: false,
                stop_threshold_pct: 100,
                start_threshold_pct: 95,
                mode: BatteryChargeMode::Auto,
                builtin: true,
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub language: String,
    pub minimize_to_tray_on_close: bool,
    pub start_minimized: bool,
    pub start_with_windows: bool,
    pub show_tray_notice_on_close: bool,
    pub dock_bottom_right: bool,
    pub hide_on_focus_loss: bool,
    pub experimental_board_id: Option<String>,
    pub first_run_completed: bool,
    pub theme: String,
    pub font_scale_pct: u8,
    pub telemetry_interval_ms: u64,
    pub auto_check_updates: bool,
    pub allow_unsigned_sha256_updates: bool,
    pub custom_theme: CustomThemeConfig,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            language: "tr".to_string(),
            minimize_to_tray_on_close: true,
            start_minimized: false,
            start_with_windows: false,
            show_tray_notice_on_close: true,
            dock_bottom_right: true,
            hide_on_focus_loss: false,
            experimental_board_id: None,
            first_run_completed: false,
            theme: "dark".to_string(),
            font_scale_pct: 100,
            telemetry_interval_ms: 1000,
            auto_check_updates: true,
            allow_unsigned_sha256_updates: false,
            custom_theme: CustomThemeConfig::preset_dark_slate(),
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
    pub guard_cpu_temp_c: u8,
    pub guard_sustain_secs: u8,
    pub sync_windows_power_mode: bool,
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
            guard_cpu_temp_c: DEFAULT_GUARD_CPU_TEMP_C,
            guard_sustain_secs: DEFAULT_GUARD_SUSTAIN_SECS,
            sync_windows_power_mode: true,
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
    pub active_profile_id: Option<String>,
    pub custom_profiles: Vec<BatteryProfile>,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: true,
            one_time_full_charge: false,
            active_profile_id: None,
            custom_profiles: Vec::new(),
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
    pub ac_fan_mode: Option<FanMode>,
    pub dc_fan_mode: Option<FanMode>,
    pub ac_curve_preset: Option<FanCurvePreset>,
    pub dc_curve_preset: Option<FanCurvePreset>,
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
            ac_fan_mode: None,
            dc_fan_mode: None,
            ac_curve_preset: None,
            dc_curve_preset: None,
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
    pub custom_args: String,
    pub global_toggle_shortcut: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            capture_omen_key: true,
            omen_key_action: OmenKeyAction::ToggleWindow,
            custom_command: String::new(),
            custom_args: String::new(),
            global_toggle_shortcut: "Ctrl+Shift+O".to_string(),
        }
    }
}

/// Splits a legacy combined `custom_command` string into `(executable_or_lnk_path, args)`.
pub fn split_legacy_command(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }

    if let Some(after_quote) = trimmed.strip_prefix('"') {
        if let Some(end_quote) = after_quote.find('"') {
            let path = after_quote[..end_quote].trim().to_string();
            let args = after_quote[end_quote + 1..].trim().to_string();
            return (path, args);
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    for ext in [".exe", ".lnk"] {
        let mut search_from = 0usize;
        while let Some(rel) = lower[search_from..].find(ext) {
            let idx = search_from + rel + ext.len();
            if idx == trimmed.len() {
                return (trimmed.to_string(), String::new());
            }
            if trimmed[idx..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_whitespace())
            {
                return (
                    trimmed[..idx].trim().to_string(),
                    trimmed[idx..].trim().to_string(),
                );
            }
            search_from = idx;
        }
    }

    match trimmed.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((head, tail)) => (head.trim().to_string(), tail.trim().to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

/// Validates syntactic safety of a custom command target path: non-empty, `.exe` or `.lnk`,
/// without control characters or shell chaining operators.
pub fn is_valid_custom_command_syntax(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed
        .chars()
        .any(|c| matches!(c, '\0' | '\r' | '\n' | '&' | '|' | ';' | '<' | '>' | '`'))
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    (lower.ends_with(".exe") && lower.len() > 4) || (lower.ends_with(".lnk") && lower.len() > 4)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(default)]
    pub config_version: u32,
    pub general: GeneralConfig,
    pub logging: LogConfig,
    pub thermal_fan: ThermalFanConfig,
    pub battery: BatteryConfig,
    pub lighting: LightingConfig,
    pub auto_profile: AutoProfileConfig,
    pub app_profiles: AppProfilesConfig,
    pub hotkeys: HotkeyConfig,
    pub tuning: TuningConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            general: GeneralConfig::default(),
            logging: LogConfig::default(),
            thermal_fan: ThermalFanConfig::default(),
            battery: BatteryConfig::default(),
            lighting: LightingConfig::default(),
            auto_profile: AutoProfileConfig::default(),
            app_profiles: AppProfilesConfig::default(),
            hotkeys: HotkeyConfig::default(),
            tuning: TuningConfig::default(),
        }
    }
}

/// Outcome of loading `config.toml`, preserving unreadable corrupt files under `.bad-<stamp>`.
#[derive(Debug, Clone)]
pub struct ConfigLoadOutcome {
    pub config: AppConfig,
    pub warnings: Vec<String>,
    pub corrupt_backup: Option<PathBuf>,
    pub load_failed: bool,
    pub load_error: Option<String>,
}

/// Builds `<file>.bad-<stamp>` in the same directory as `path`.
pub fn corrupt_backup_path(path: &Path, stamp: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".to_string());
    let candidate = format!("{file_name}.bad-{stamp}");
    match path.parent() {
        Some(parent) => parent.join(candidate),
        None => PathBuf::from(candidate),
    }
}

fn allocate_corrupt_backup_path(path: &Path, stamp: &str) -> PathBuf {
    let primary = corrupt_backup_path(path, stamp);
    if !primary.exists() {
        return primary;
    }
    for idx in 1..=99u32 {
        let candidate = corrupt_backup_path(path, &format!("{stamp}-{idx}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    primary
}

impl AppConfig {
    pub fn default_config_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return PathBuf::from(app_data).join("OmvixHub").join("config.toml");
        }

        if let Some(dirs) = ProjectDirs::from("io", "omvix", "OmvixHub") {
            return dirs.config_dir().join("config.toml");
        }

        PathBuf::from("omvix-config.toml")
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let (cfg, _warnings) = Self::load_from_path_with_warnings(path)?;
        Ok(cfg)
    }

    /// Loads `config.toml`, quarantining unreadable corrupt files to `config.toml.bad-<YYYYMMDD-HHMMSS>`
    /// so user edits are never silently wiped.
    pub fn load_or_quarantine_corrupt(path: &Path) -> ConfigLoadOutcome {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        Self::load_or_quarantine_corrupt_with_stamp(path, &stamp)
    }

    pub fn load_or_quarantine_corrupt_with_stamp(path: &Path, stamp: &str) -> ConfigLoadOutcome {
        match Self::load_from_path_with_warnings(path) {
            Ok((config, warnings)) => ConfigLoadOutcome {
                config,
                warnings,
                corrupt_backup: None,
                load_failed: false,
                load_error: None,
            },
            Err(err) => {
                let err_msg = err.to_string();
                if path.exists() {
                    let backup = allocate_corrupt_backup_path(path, stamp);
                    match fs::rename(path, &backup) {
                        Ok(()) => ConfigLoadOutcome {
                            config: Self::default(),
                            warnings: vec![format!(
                                "Corrupt config.toml was backed up to {} and defaults were loaded ({err_msg}).",
                                backup.display()
                            )],
                            corrupt_backup: Some(backup),
                            load_failed: true,
                            load_error: Some(err_msg),
                        },
                        Err(rename_err) => ConfigLoadOutcome {
                            config: Self::default(),
                            warnings: vec![format!(
                                "Failed reading config.toml ({err_msg}); backup rename failed ({rename_err}), leaving original untouched."
                            )],
                            corrupt_backup: None,
                            load_failed: true,
                            load_error: Some(format!("{err_msg}; rename error: {rename_err}")),
                        },
                    }
                } else {
                    ConfigLoadOutcome {
                        config: Self::default(),
                        warnings: vec![format!(
                            "Failed reading config, restored defaults: {err_msg}"
                        )],
                        corrupt_backup: None,
                        load_failed: true,
                        load_error: Some(err_msg),
                    }
                }
            }
        }
    }

    pub fn load_from_path_with_warnings(path: &Path) -> Result<(Self, Vec<String>)> {
        if !path.exists() {
            return Ok((Self::default(), Vec::new()));
        }
        let raw = fs::read_to_string(path)?;
        let mut cfg: Self = toml::from_str(&raw).map_err(|e| OmvixError::Config(e.to_string()))?;
        if cfg.config_version < 2 {
            // Migrate pre-v0.4.2 configs once to enable safe shutdown Hold by default.
            cfg.battery.hold_when_shutdown = true;
        }
        if cfg.config_version < 3 {
            // Migrate pre-v0.4.5 configs to configurable sustained thermal guard defaults.
            cfg.thermal_fan.guard_cpu_temp_c = DEFAULT_GUARD_CPU_TEMP_C;
            cfg.thermal_fan.guard_sustain_secs = DEFAULT_GUARD_SUSTAIN_SECS;
        }
        if cfg.config_version < 4 {
            // Existing pre-v0.5 users already configured Omvix Hub; keep wizard accessible from Settings.
            cfg.general.first_run_completed = true;
        }
        cfg.config_version = CURRENT_CONFIG_VERSION;
        let warnings = cfg.sanitize();
        Ok((cfg, warnings))
    }

    /// Validates, clamps, and migrates untrusted configuration values loaded from disk.
    /// Returns human-readable warnings/migration notes to log and surface in the UI.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        self.general.telemetry_interval_ms = self.general.telemetry_interval_ms.clamp(100, 10_000);
        self.general.font_scale_pct = self.general.font_scale_pct.clamp(85, 130);
        self.logging.max_total_mb = self.logging.max_total_mb.clamp(5, 100);
        self.logging.max_files = self.logging.max_files.clamp(1, 16);

        self.thermal_fan.manual_speed_pct = self.thermal_fan.manual_speed_pct.clamp(0, 100);
        self.thermal_fan.keep_alive_interval_secs =
            self.thermal_fan.keep_alive_interval_secs.clamp(15, 110);
        self.thermal_fan.guard_cpu_temp_c = self.thermal_fan.guard_cpu_temp_c.clamp(90, 98);
        self.thermal_fan.guard_sustain_secs = self.thermal_fan.guard_sustain_secs.clamp(10, 30);

        let had_too_few = self.thermal_fan.custom_curve.points.len() < 2;
        if self.thermal_fan.custom_curve.normalize() {
            if had_too_few {
                warnings.push(
                    "Custom fan curve had fewer than 2 points; restored default curve points."
                        .to_string(),
                );
            } else {
                warnings.push(
                    "Normalized custom fan curve (monotonic non-decreasing & >= 50% at >= 80°C)."
                        .to_string(),
                );
            }
        }

        self.battery.stop_threshold_pct = self.battery.stop_threshold_pct.clamp(50, 100);
        let max_start = self.battery.stop_threshold_pct.saturating_sub(2).max(40);
        self.battery.start_threshold_pct = self.battery.start_threshold_pct.clamp(40, max_start);
        for prof in &mut self.battery.custom_profiles {
            prof.stop_threshold_pct = prof.stop_threshold_pct.clamp(50, 100);
            let p_max_start = prof.stop_threshold_pct.saturating_sub(2).max(40);
            prof.start_threshold_pct = prof.start_threshold_pct.clamp(40, p_max_start);
        }

        self.lighting.brightness_pct = self.lighting.brightness_pct.clamp(0, 100);

        if let Some(ref mut pl1) = self.tuning.pl1_watts {
            *pl1 = (*pl1).clamp(15, 200);
        }
        if let Some(ref mut pl2) = self.tuning.pl2_watts {
            *pl2 = (*pl2).clamp(15, 200);
        }
        if let Some(ref mut tcc) = self.tuning.tcc_temp_limit_c {
            *tcc = (*tcc).clamp(70, 100);
        }
        if let Some(ref mut uv) = self.tuning.core_undervolt_mv {
            *uv = (*uv).clamp(-150, 0);
        }

        let cleaned_args: String = self
            .hotkeys
            .custom_args
            .chars()
            .filter(|c| !matches!(c, '\0' | '\r' | '\n'))
            .collect();
        self.hotkeys.custom_args = cleaned_args.trim().to_string();

        if !self.hotkeys.custom_command.trim().is_empty() {
            let original = self.hotkeys.custom_command.trim().to_string();
            let (exe_part, split_args) = split_legacy_command(&original);
            if is_valid_custom_command_syntax(&exe_part) {
                self.hotkeys.custom_command = exe_part;
                if !split_args.is_empty() {
                    if self.hotkeys.custom_args.is_empty() {
                        self.hotkeys.custom_args = split_args;
                    } else {
                        self.hotkeys.custom_args =
                            format!("{split_args} {}", self.hotkeys.custom_args);
                    }
                    warnings.push(format!(
                        "Migrated legacy custom_command arguments into custom_args ({}).",
                        self.hotkeys.custom_command
                    ));
                }
            } else {
                self.hotkeys.custom_command.clear();
                self.hotkeys.custom_args.clear();
                warnings.push(format!(
                    "Cleared invalid custom_command for security (only .exe/.lnk allowed): {original}"
                ));
            }
        }

        self.app_profiles.poll_interval_secs = self.app_profiles.poll_interval_secs.clamp(2, 15);
        let mut sanitized_rules = Vec::with_capacity(self.app_profiles.rules.len());
        for (idx, mut rule) in self.app_profiles.rules.drain(..).enumerate() {
            match normalize_exe_name(&rule.exe_name) {
                Some(norm) => {
                    rule.exe_name = norm;
                    if rule.id.trim().is_empty() {
                        rule.id = format!("rule-{}", idx + 1);
                    }
                    sanitized_rules.push(rule);
                }
                None => {
                    warnings.push(format!(
                        "Removed invalid app profile rule with executable '{}'",
                        rule.exe_name
                    ));
                }
            }
        }
        self.app_profiles.rules = sanitized_rules;

        warnings
    }

    /// Saves configuration atomically via a temporary file + rename.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let serialized =
            toml::to_string_pretty(self).map_err(|e| OmvixError::Config(e.to_string()))?;
        if path.exists() && fs::read_to_string(path).ok().as_deref() == Some(serialized.as_str()) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("toml.tmp");
        fs::write(&tmp_path, serialized)?;
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
        cfg.general.telemetry_interval_ms = 100;
        cfg.general.dock_bottom_right = false;
        cfg.general.hide_on_focus_loss = true;
        cfg.general.custom_theme = CustomThemeConfig::preset_omen_crimson();
        cfg.battery.enabled = true;
        cfg.battery.stop_threshold_pct = 80;
        cfg.battery.start_threshold_pct = 75;
        cfg.battery.active_profile_id = Some("balanced".to_string());
        cfg.battery.custom_profiles.push(BatteryProfile {
            id: "gaming-cap".to_string(),
            name: "Gaming Cap".to_string(),
            enabled: true,
            stop_threshold_pct: 70,
            start_threshold_pct: 65,
            mode: BatteryChargeMode::Hold,
            builtin: false,
        });
        cfg.logging.max_total_mb = 20;

        cfg.save_to_path(&path).expect("save config");
        let loaded = AppConfig::load_from_path(&path).expect("load config");

        assert_eq!(loaded.general.language, "en");
        assert_eq!(loaded.general.telemetry_interval_ms, 100);
        assert!(!loaded.general.dock_bottom_right);
        assert!(loaded.general.hide_on_focus_loss);
        assert_eq!(
            loaded.general.custom_theme,
            CustomThemeConfig::preset_omen_crimson()
        );
        assert!(loaded.battery.enabled);
        assert_eq!(loaded.battery.stop_threshold_pct, 80);
        assert_eq!(loaded.battery.start_threshold_pct, 75);
        assert_eq!(
            loaded.battery.active_profile_id.as_deref(),
            Some("balanced")
        );
        assert_eq!(loaded.battery.custom_profiles.len(), 1);
        assert_eq!(loaded.battery.custom_profiles[0].name, "Gaming Cap");
        assert_eq!(loaded.logging.max_total_mb, 20);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_share_code_and_json_roundtrip() {
        for preset in CustomThemeConfig::presets() {
            let code = preset.encode_share_code();
            let decoded = CustomThemeConfig::decode_share_code(&code).expect("decode compact code");
            assert_eq!(decoded, preset);

            let json = serde_json::to_string(&preset).expect("serialize theme json");
            let from_json = CustomThemeConfig::decode_share_code(&json).expect("decode json code");
            assert_eq!(from_json, preset);
        }

        assert!(CustomThemeConfig::decode_share_code("").is_err());
        assert!(CustomThemeConfig::decode_share_code("INVALID|D|000000").is_err());
        assert!(CustomThemeConfig::decode_share_code(
            "OMVIX1|D|zzzzzz|000000|000000|000000|000000|ffffff|Bad"
        )
        .is_err());
    }

    #[test]
    fn detects_legacy_bluish_dark_slate() {
        let old = CustomThemeConfig::decode_share_code(
            "OMVIX1|D|00b4d8|0f141b|0c1118|161d27|263140|e6edf3|Dark Slate",
        )
        .unwrap();
        assert!(old.is_legacy_bluish_dark_slate());
        assert!(!CustomThemeConfig::preset_dark_slate().is_legacy_bluish_dark_slate());
        assert!(!CustomThemeConfig::preset_omen_crimson().is_legacy_bluish_dark_slate());
    }

    #[test]
    fn battery_hold_default_is_true_and_legacy_config_migrates_once() {
        assert!(BatteryConfig::default().hold_when_shutdown);
        let fresh = AppConfig::default();
        assert_eq!(fresh.config_version, CURRENT_CONFIG_VERSION);
        assert!(!fresh.general.first_run_completed);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_migrate_test_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        // Pre-v0.4.2 config had no `config_version` and `hold_when_shutdown = false` -> migrates to true + first_run_completed = true
        let legacy_toml = "[battery]\nenabled = true\nhold_when_shutdown = false\n";
        fs::write(&path, legacy_toml).unwrap();
        let migrated = AppConfig::load_from_path(&path).expect("load legacy");
        assert_eq!(migrated.config_version, CURRENT_CONFIG_VERSION);
        assert!(migrated.battery.hold_when_shutdown);
        assert!(migrated.general.first_run_completed);

        // v2 config where user explicitly chose false -> preserves false, migrates thermal guard + first_run_completed
        let v2_toml = "config_version = 2\n[battery]\nenabled = true\nhold_when_shutdown = false\n";
        fs::write(&path, v2_toml).unwrap();
        let explicit = AppConfig::load_from_path(&path).expect("load v2");
        assert_eq!(explicit.config_version, CURRENT_CONFIG_VERSION);
        assert!(!explicit.battery.hold_when_shutdown);
        assert_eq!(
            explicit.thermal_fan.guard_cpu_temp_c,
            DEFAULT_GUARD_CPU_TEMP_C
        );
        assert_eq!(
            explicit.thermal_fan.guard_sustain_secs,
            DEFAULT_GUARD_SUSTAIN_SECS
        );
        assert!(explicit.general.first_run_completed);

        // v3 config preserves explicit guard overrides and marks first_run_completed for existing users
        let v3_toml =
            "config_version = 3\n[thermal_fan]\nguard_cpu_temp_c = 92\nguard_sustain_secs = 15\n";
        fs::write(&path, v3_toml).unwrap();
        let v3_loaded = AppConfig::load_from_path(&path).expect("load v3");
        assert_eq!(v3_loaded.config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(v3_loaded.thermal_fan.guard_cpu_temp_c, 92);
        assert_eq!(v3_loaded.thermal_fan.guard_sustain_secs, 15);
        assert!(v3_loaded.general.first_run_completed);

        // Explicit v4 config preserves first_run_completed = false and explicit guard overrides
        let v4_toml = format!(
            "config_version = {CURRENT_CONFIG_VERSION}\n[general]\nfirst_run_completed = false\n[thermal_fan]\nguard_cpu_temp_c = 92\nguard_sustain_secs = 15\n"
        );
        fs::write(&path, v4_toml).unwrap();
        let v4_loaded = AppConfig::load_from_path(&path).expect("load v4");
        assert!(!v4_loaded.general.first_run_completed);
        assert_eq!(v4_loaded.thermal_fan.guard_cpu_temp_c, 92);
        assert_eq!(v4_loaded.thermal_fan.guard_sustain_secs, 15);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_enforces_monotonic_curve_and_high_temp_floor_with_warning() {
        let mut cfg = AppConfig::default();
        cfg.thermal_fan.guard_cpu_temp_c = 80;
        cfg.thermal_fan.guard_sustain_secs = 2;
        cfg.thermal_fan.custom_curve.points = vec![
            crate::models::FanCurvePoint {
                temp_c: 40,
                speed_pct: 35,
            },
            crate::models::FanCurvePoint {
                temp_c: 65,
                speed_pct: 20,
            },
            crate::models::FanCurvePoint {
                temp_c: 85,
                speed_pct: 10,
            },
        ];
        let warnings = cfg.sanitize();
        assert!(!warnings.is_empty());
        assert_eq!(cfg.thermal_fan.guard_cpu_temp_c, 90);
        assert_eq!(cfg.thermal_fan.guard_sustain_secs, 10);
        assert_eq!(cfg.thermal_fan.custom_curve.points[1].speed_pct, 35);
        assert_eq!(cfg.thermal_fan.custom_curve.points[2].speed_pct, 50);
    }

    #[test]
    fn save_to_path_skips_redundant_identical_write() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_noop_save_test_{nanos}"));
        let path = dir.join("config.toml");

        let cfg = AppConfig::default();
        cfg.save_to_path(&path).expect("initial save");
        let first_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(25));
        cfg.save_to_path(&path)
            .expect("identical save should succeed");
        let second_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        assert_eq!(first_mtime, second_mtime);
        assert!(!path.with_extension("toml.tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_to_path_overwrites_atomically_without_leaving_tmp() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_atomic_save_test_{nanos}"));
        let path = dir.join("config.toml");

        let mut cfg = AppConfig::default();
        cfg.thermal_fan.manual_speed_pct = 45;
        cfg.save_to_path(&path).expect("first save");

        cfg.thermal_fan.manual_speed_pct = 75;
        cfg.save_to_path(&path).expect("atomic overwrite");

        assert!(!path.with_extension("toml.tmp").exists());
        let reloaded = AppConfig::load_from_path(&path).expect("reload");
        assert_eq!(reloaded.thermal_fan.manual_speed_pct, 75);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_migrates_legacy_custom_command_and_warns_on_invalid() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.custom_command = r#""C:\Program Files\MyApp\tool.exe" --tray --fast"#.into();
        let w1 = cfg.sanitize();
        assert_eq!(
            cfg.hotkeys.custom_command,
            r"C:\Program Files\MyApp\tool.exe"
        );
        assert_eq!(cfg.hotkeys.custom_args, "--tray --fast");
        assert_eq!(w1.len(), 1);

        cfg.hotkeys.custom_command = "cmd /C calc && whoami".into();
        cfg.hotkeys.custom_args = "--old".into();
        let w2 = cfg.sanitize();
        assert!(cfg.hotkeys.custom_command.is_empty());
        assert!(cfg.hotkeys.custom_args.is_empty());
        assert!(!w2.is_empty());

        cfg.hotkeys.custom_command = r"C:\Users\Public\Shortcuts\Game.lnk".into();
        let w3 = cfg.sanitize();
        assert!(w3.is_empty());
        assert_eq!(
            cfg.hotkeys.custom_command,
            r"C:\Users\Public\Shortcuts\Game.lnk"
        );
    }

    #[test]
    fn sanitize_clamps_out_of_range_curve_thresholds_and_intervals() {
        let mut cfg = AppConfig::default();
        cfg.general.telemetry_interval_ms = 5;
        cfg.logging.max_total_mb = 500;
        cfg.battery.stop_threshold_pct = 42;
        cfg.battery.start_threshold_pct = 90;
        cfg.thermal_fan.manual_speed_pct = 200;
        cfg.thermal_fan.custom_curve.hysteresis_c = 99;
        cfg.thermal_fan.custom_curve.points = vec![crate::models::FanCurvePoint {
            temp_c: 95,
            speed_pct: 50,
        }];
        cfg.tuning.pl1_watts = Some(1);
        cfg.tuning.core_undervolt_mv = Some(-500);

        let warnings = cfg.sanitize();
        assert!(!warnings.is_empty());
        assert_eq!(cfg.general.telemetry_interval_ms, 100);
        assert_eq!(cfg.logging.max_total_mb, 100);
        assert_eq!(cfg.battery.stop_threshold_pct, 50);
        assert_eq!(cfg.battery.start_threshold_pct, 48);
        assert_eq!(cfg.thermal_fan.manual_speed_pct, 100);
        assert_eq!(cfg.thermal_fan.custom_curve.hysteresis_c, 10);
        assert_eq!(cfg.thermal_fan.custom_curve.points.len(), 8);
        assert_eq!(cfg.tuning.pl1_watts, Some(15));
        assert_eq!(cfg.tuning.core_undervolt_mv, Some(-150));
    }

    #[test]
    fn app_profiles_and_enhanced_auto_profile_roundtrip_and_sanitize() {
        use crate::app_profiles::AppProfileRule;

        let mut cfg = AppConfig::default();
        assert!(cfg.thermal_fan.sync_windows_power_mode);

        cfg.thermal_fan.sync_windows_power_mode = false;
        cfg.auto_profile.enabled = true;
        cfg.auto_profile.ac_fan_mode = Some(FanMode::CustomCurve);
        cfg.auto_profile.ac_curve_preset = Some(FanCurvePreset::Aggressive);
        cfg.auto_profile.dc_fan_mode = Some(FanMode::Auto);
        cfg.app_profiles.enabled = true;
        cfg.app_profiles.poll_interval_secs = 99;
        cfg.app_profiles.rules.push(AppProfileRule {
            id: String::new(),
            name: "Blender".into(),
            exe_name: r#""C:\Program Files\Blender\Blender.EXE""#.into(),
            enabled: true,
            performance_mode: PerformanceMode::Performance,
            fan_mode: Some(FanMode::CustomCurve),
            curve_preset: Some(FanCurvePreset::OghLike),
            refresh_hz: Some(165),
            cpu_boost_mode: Some(CpuBoostMode::Aggressive),
        });
        cfg.app_profiles.rules.push(AppProfileRule {
            id: "bad".into(),
            name: "Invalid".into(),
            exe_name: "..".into(),
            ..Default::default()
        });

        let warnings = cfg.sanitize();
        assert_eq!(cfg.app_profiles.poll_interval_secs, 15);
        assert_eq!(cfg.app_profiles.rules.len(), 1);
        assert_eq!(cfg.app_profiles.rules[0].exe_name, "blender.exe");
        assert_eq!(cfg.app_profiles.rules[0].id, "rule-1");
        assert_eq!(warnings.len(), 1);

        let serialized = toml::to_string_pretty(&cfg).expect("serialize");
        let parsed: AppConfig = toml::from_str(&serialized).expect("deserialize");
        assert!(!parsed.thermal_fan.sync_windows_power_mode);
        assert_eq!(parsed.auto_profile.ac_fan_mode, Some(FanMode::CustomCurve));
        assert_eq!(
            parsed.auto_profile.ac_curve_preset,
            Some(FanCurvePreset::Aggressive)
        );
        assert_eq!(parsed.app_profiles, cfg.app_profiles);

        // Older TOML lacking new fields deserializes cleanly with defaults
        let older: AppConfig =
            toml::from_str("[thermal_fan]\nperformance_mode = \"Eco\"\n").unwrap();
        assert!(older.thermal_fan.sync_windows_power_mode);
        assert!(!older.app_profiles.enabled);
        assert_eq!(older.app_profiles.poll_interval_secs, 3);
        assert_eq!(older.general.font_scale_pct, 100);
    }

    #[test]
    fn accessibility_font_scale_clamps_and_high_contrast_preset_detected() {
        let mut cfg = AppConfig::default();
        assert_eq!(cfg.general.font_scale_pct, 100);
        cfg.general.font_scale_pct = 50;
        let _ = cfg.sanitize();
        assert_eq!(cfg.general.font_scale_pct, 85);
        cfg.general.font_scale_pct = 220;
        let _ = cfg.sanitize();
        assert_eq!(cfg.general.font_scale_pct, 130);

        let hc = CustomThemeConfig::preset_high_contrast();
        assert!(hc.is_high_contrast());
        assert!(!CustomThemeConfig::preset_dark_slate().is_high_contrast());
        assert!(CustomThemeConfig::presets().contains(&hc));
    }

    #[test]
    fn updater_config_defaults_fail_closed_and_roundtrip() {
        let mut cfg = AppConfig::default();
        assert!(cfg.general.auto_check_updates);
        assert!(!cfg.general.allow_unsigned_sha256_updates);

        cfg.general.auto_check_updates = false;
        cfg.general.allow_unsigned_sha256_updates = true;
        let toml_text = toml::to_string_pretty(&cfg).expect("serialize");
        let parsed: AppConfig = toml::from_str(&toml_text).expect("deserialize");
        assert!(!parsed.general.auto_check_updates);
        assert!(parsed.general.allow_unsigned_sha256_updates);
    }

    #[test]
    fn corrupt_config_is_preserved_in_bad_backup_file() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_corrupt_cfg_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let corrupt_payload = "[general\nlanguage = \"broken_toml_missing_bracket\"\n";
        fs::write(&path, corrupt_payload).unwrap();

        let outcome = AppConfig::load_or_quarantine_corrupt_with_stamp(&path, "20261122-151500");
        assert!(outcome.load_failed);
        assert!(outcome.load_error.is_some());
        let expected_backup = dir.join("config.toml.bad-20261122-151500");
        assert_eq!(outcome.corrupt_backup.as_ref(), Some(&expected_backup));
        assert!(expected_backup.is_file());
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(&expected_backup).unwrap(),
            corrupt_payload
        );
        assert!(!outcome.warnings.is_empty());
        assert!(outcome.warnings[0].contains("config.toml.bad-20261122-151500"));
        assert_eq!(
            outcome.config.general.language,
            AppConfig::default().general.language
        );

        // Collision on the same stamp allocates `-1` without clobbering existing backup
        fs::write(&path, "not = [valid").unwrap();
        let outcome2 = AppConfig::load_or_quarantine_corrupt_with_stamp(&path, "20261122-151500");
        let expected_backup_2 = dir.join("config.toml.bad-20261122-151500-1");
        assert_eq!(outcome2.corrupt_backup.as_ref(), Some(&expected_backup_2));
        assert_eq!(
            fs::read_to_string(&expected_backup).unwrap(),
            corrupt_payload
        );
        assert_eq!(
            fs::read_to_string(&expected_backup_2).unwrap(),
            "not = [valid"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_config_with_sanitize_warning_does_not_create_bad_backup() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_valid_warn_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let mut cfg = AppConfig::default();
        cfg.hotkeys.custom_command = "cmd.exe /c whoami".to_string();
        cfg.save_to_path(&path).unwrap();

        let outcome = AppConfig::load_or_quarantine_corrupt_with_stamp(&path, "20261122-151500");
        assert!(!outcome.load_failed);
        assert!(outcome.corrupt_backup.is_none());
        assert!(!outcome.warnings.is_empty());
        assert!(!dir.join("config.toml.bad-20261122-151500").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
