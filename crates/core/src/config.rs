use crate::error::{OmvixError, Result};
use crate::logging::LogConfig;
use crate::models::{
    BatteryChargeMode, CpuBoostMode, FanCurve, FanMode, GpuMode, PerformanceMode, RgbColor,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const CURRENT_CONFIG_VERSION: u32 = 2;

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

    pub fn presets() -> Vec<Self> {
        vec![
            Self::preset_dark_slate(),
            Self::preset_clean_light(),
            Self::preset_omen_crimson(),
            Self::preset_emerald_matrix(),
            Self::preset_midnight_violet(),
        ]
    }

    /// Encodes this theme into a compact shareable ASCII code (`OMVIX1|...`).
    pub fn encode_share_code(&self) -> String {
        let mode = if self.dark_base { "D" } else { "L" };
        let clean_name = self
            .name
            .replace(['|', '\r', '\n'], " ")
            .trim()
            .to_string();
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
    pub theme: String,
    pub telemetry_interval_ms: u64,
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
            theme: "dark".to_string(),
            telemetry_interval_ms: 1000,
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
            hotkeys: HotkeyConfig::default(),
            tuning: TuningConfig::default(),
        }
    }
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
        let mut cfg: Self = toml::from_str(&raw).map_err(|e| OmvixError::Config(e.to_string()))?;
        if cfg.config_version < CURRENT_CONFIG_VERSION {
            // Migrate pre-v0.4.2 configs once to enable safe shutdown Hold by default.
            cfg.battery.hold_when_shutdown = true;
            cfg.config_version = CURRENT_CONFIG_VERSION;
        }
        Ok(cfg)
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
        assert_eq!(loaded.battery.active_profile_id.as_deref(), Some("balanced"));
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
        assert!(CustomThemeConfig::decode_share_code("OMVIX1|D|zzzzzz|000000|000000|000000|000000|ffffff|Bad").is_err());
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
        assert_eq!(AppConfig::default().config_version, CURRENT_CONFIG_VERSION);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_migrate_test_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        // Pre-v0.4.2 config had no `config_version` and `hold_when_shutdown = false` -> migrates to true
        let legacy_toml = "[battery]\nenabled = true\nhold_when_shutdown = false\n";
        fs::write(&path, legacy_toml).unwrap();
        let migrated = AppConfig::load_from_path(&path).expect("load legacy");
        assert_eq!(migrated.config_version, CURRENT_CONFIG_VERSION);
        assert!(migrated.battery.hold_when_shutdown);

        // v2 config where user explicitly chose false -> preserves false
        let v2_toml = format!(
            "config_version = {CURRENT_CONFIG_VERSION}\n[battery]\nenabled = true\nhold_when_shutdown = false\n"
        );
        fs::write(&path, v2_toml).unwrap();
        let explicit = AppConfig::load_from_path(&path).expect("load v2");
        assert_eq!(explicit.config_version, CURRENT_CONFIG_VERSION);
        assert!(!explicit.battery.hold_when_shutdown);

        let _ = fs::remove_dir_all(&dir);
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
        cfg.save_to_path(&path).expect("identical save should succeed");
        let second_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        assert_eq!(first_mtime, second_mtime);
        assert!(!path.with_extension("toml.tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
