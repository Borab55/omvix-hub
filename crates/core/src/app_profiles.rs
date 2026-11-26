use crate::auto_profile::AutoProfileAction;
use crate::models::{CpuBoostMode, FanCurve, FanCurvePreset, FanMode, PerformanceMode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppProfileRule {
    pub id: String,
    pub name: String,
    pub exe_name: String,
    pub enabled: bool,
    pub performance_mode: PerformanceMode,
    pub fan_mode: Option<FanMode>,
    pub curve_preset: Option<FanCurvePreset>,
    pub refresh_hz: Option<u32>,
    pub cpu_boost_mode: Option<CpuBoostMode>,
}

impl Default for AppProfileRule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            exe_name: String::new(),
            enabled: true,
            performance_mode: PerformanceMode::Performance,
            fan_mode: None,
            curve_preset: None,
            refresh_hz: None,
            cpu_boost_mode: None,
        }
    }
}

impl AppProfileRule {
    pub fn display_label(&self) -> &str {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            &self.exe_name
        } else {
            trimmed
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppProfilesConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub rules: Vec<AppProfileRule>,
}

impl Default for AppProfilesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: 3,
            rules: Vec::new(),
        }
    }
}

/// Normalizes a user-supplied executable name or path into a lowercase filename (e.g. `game.exe`).
/// Returns `None` for empty strings, relative traversal tokens, or control characters.
pub fn normalize_exe_name(raw: &str) -> Option<String> {
    if raw.chars().any(char::is_control) {
        return None;
    }
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return None;
    }

    let basename = trimmed
        .rsplit(['\\', '/'])
        .next()
        .map(|s| s.trim().trim_matches('"').trim())
        .unwrap_or("");
    if basename.is_empty() || basename == "." || basename == ".." {
        return None;
    }

    let mut lower = basename.to_ascii_lowercase();
    if lower.ends_with(' ')
        || lower
            .chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return None;
    }
    if !lower.contains('.') {
        lower.push_str(".exe");
    }
    if lower == "." || lower == ".." || lower.starts_with('.') || lower.ends_with('.') {
        return None;
    }

    Some(lower)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBaseline {
    pub performance_mode: PerformanceMode,
    pub fan_mode: FanMode,
    pub gpu_fan_mode: FanMode,
    pub manual_speed_pct: u8,
    pub gpu_manual_speed_pct: u8,
    pub custom_curve: FanCurve,
    pub gpu_custom_curve: FanCurve,
    pub link_cpu_gpu_fans: bool,
    pub refresh_hz: Option<u32>,
    pub cpu_boost_mode: Option<CpuBoostMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppProfileTransition {
    None,
    Activate { rule: AppProfileRule },
    Restore(ProfileBaseline),
}

/// Matches running `.exe` names against configured priority-ordered rules and captures/restores
/// the user's pre-override hardware state cleanly once matched apps exit.
#[derive(Debug, Clone, Default)]
pub struct AppProfileEngine {
    active_rule: Option<AppProfileRule>,
    saved_baseline: Option<ProfileBaseline>,
    exit_miss_count: u8,
}

impl AppProfileEngine {
    const EXIT_CONFIRM_POLLS: u8 = 2;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_rule(&self) -> Option<&AppProfileRule> {
        self.active_rule.as_ref()
    }

    pub fn saved_baseline(&self) -> Option<&ProfileBaseline> {
        self.saved_baseline.as_ref()
    }

    pub fn reset(&mut self) {
        self.active_rule = None;
        self.saved_baseline = None;
        self.exit_miss_count = 0;
    }

    pub fn update_baseline_for_ac_dc(&mut self, action: &AutoProfileAction) {
        if let Some(ref mut baseline) = self.saved_baseline {
            baseline.performance_mode = action.performance_mode;
            if let Some(fm) = action.fan_mode {
                baseline.fan_mode = fm;
                baseline.gpu_fan_mode = fm;
            }
            if let Some(preset) = action.curve_preset {
                let curve = preset.to_curve();
                baseline.custom_curve = curve.clone();
                baseline.gpu_custom_curve = curve;
            }
            if action.refresh_hz.is_some() {
                baseline.refresh_hz = action.refresh_hz;
            }
        }
    }

    pub fn evaluate<F>(
        &mut self,
        cfg: &AppProfilesConfig,
        running_exes_lower: &HashSet<String>,
        capture_baseline: F,
    ) -> AppProfileTransition
    where
        F: FnOnce() -> ProfileBaseline,
    {
        if !cfg.enabled {
            self.exit_miss_count = 0;
            if self.active_rule.take().is_some() {
                if let Some(baseline) = self.saved_baseline.take() {
                    return AppProfileTransition::Restore(baseline);
                }
            }
            return AppProfileTransition::None;
        }

        let matched = cfg
            .rules
            .iter()
            .find(|r| r.enabled && running_exes_lower.contains(&r.exe_name.to_ascii_lowercase()));

        if let Some(rule) = matched {
            self.exit_miss_count = 0;
            if self.active_rule.as_ref() == Some(rule) {
                return AppProfileTransition::None;
            }
            if self.saved_baseline.is_none() {
                self.saved_baseline = Some(capture_baseline());
            }
            self.active_rule = Some(rule.clone());
            return AppProfileTransition::Activate { rule: rule.clone() };
        }

        if self.active_rule.is_some() {
            self.exit_miss_count = self.exit_miss_count.saturating_add(1);
            if self.exit_miss_count >= Self::EXIT_CONFIRM_POLLS {
                self.active_rule = None;
                self.exit_miss_count = 0;
                if let Some(baseline) = self.saved_baseline.take() {
                    return AppProfileTransition::Restore(baseline);
                }
            }
        }

        AppProfileTransition::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_baseline() -> ProfileBaseline {
        ProfileBaseline {
            performance_mode: PerformanceMode::Balanced,
            fan_mode: FanMode::Auto,
            gpu_fan_mode: FanMode::Auto,
            manual_speed_pct: 50,
            gpu_manual_speed_pct: 50,
            custom_curve: FanCurve::default(),
            gpu_custom_curve: FanCurve::default(),
            link_cpu_gpu_fans: false,
            refresh_hz: Some(165),
            cpu_boost_mode: Some(CpuBoostMode::Aggressive),
        }
    }

    fn set_of(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalize_exe_name_handles_paths_quotes_and_rejects_invalid() {
        assert_eq!(
            normalize_exe_name(r#"C:\Games\Cyberpunk\bin\x64\Cyberpunk2077.exe"#),
            Some("cyberpunk2077.exe".to_string())
        );
        assert_eq!(
            normalize_exe_name(r#"  "D:\Apps\Blender\Blender.EXE"  "#),
            Some("blender.exe".to_string())
        );
        assert_eq!(
            normalize_exe_name("Valorant"),
            Some("valorant.exe".to_string())
        );
        assert_eq!(normalize_exe_name(""), None);
        assert_eq!(normalize_exe_name("   "), None);
        assert_eq!(normalize_exe_name("."), None);
        assert_eq!(normalize_exe_name(".."), None);
        assert_eq!(normalize_exe_name(".exe"), None);
        assert_eq!(normalize_exe_name("bad."), None);
        assert_eq!(normalize_exe_name("game\n.exe"), None);
        assert_eq!(normalize_exe_name("game\0.exe"), None);
        assert_eq!(normalize_exe_name("bad:name.exe"), None);
        assert_eq!(normalize_exe_name("bad*.exe"), None);
        assert_eq!(normalize_exe_name("bad?.exe"), None);
    }

    #[test]
    fn activates_debounces_exit_and_restores_baseline() {
        let mut engine = AppProfileEngine::new();
        let rule = AppProfileRule {
            id: "r1".into(),
            name: "Game".into(),
            exe_name: "game.exe".into(),
            enabled: true,
            performance_mode: PerformanceMode::Performance,
            fan_mode: Some(FanMode::CustomCurve),
            curve_preset: Some(FanCurvePreset::Aggressive),
            refresh_hz: Some(165),
            cpu_boost_mode: Some(CpuBoostMode::Aggressive),
        };
        let cfg = AppProfilesConfig {
            enabled: true,
            poll_interval_secs: 3,
            rules: vec![rule.clone()],
        };

        // Empty processes -> None
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["chrome.exe"]), sample_baseline),
            AppProfileTransition::None
        );

        // Matched -> Activate
        let act = engine.evaluate(&cfg, &set_of(&["chrome.exe", "game.exe"]), sample_baseline);
        assert_eq!(act, AppProfileTransition::Activate { rule: rule.clone() });
        assert_eq!(engine.active_rule(), Some(&rule));

        // Steady state -> None
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["game.exe"]), sample_baseline),
            AppProfileTransition::None
        );

        // Miss #1 -> wait (debounce)
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["chrome.exe"]), sample_baseline),
            AppProfileTransition::None
        );
        assert!(engine.active_rule().is_some());

        // Miss #2 -> Restore baseline
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["chrome.exe"]), sample_baseline),
            AppProfileTransition::Restore(sample_baseline())
        );
        assert!(engine.active_rule().is_none());
    }

    #[test]
    fn switches_between_priority_rules_preserving_original_baseline() {
        let mut engine = AppProfileEngine::new();
        let high = AppProfileRule {
            id: "high".into(),
            name: "Heavy Game".into(),
            exe_name: "heavy.exe".into(),
            performance_mode: PerformanceMode::Performance,
            ..Default::default()
        };
        let low = AppProfileRule {
            id: "low".into(),
            name: "Browser".into(),
            exe_name: "browser.exe".into(),
            performance_mode: PerformanceMode::Quiet,
            ..Default::default()
        };
        let cfg = AppProfilesConfig {
            enabled: true,
            poll_interval_secs: 3,
            rules: vec![high.clone(), low.clone()],
        };

        // Start with low priority app -> captures Balanced baseline
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["browser.exe"]), sample_baseline),
            AppProfileTransition::Activate { rule: low.clone() }
        );

        // Launch higher-priority app -> switches without overwriting Balanced baseline
        let mut alt_baseline = sample_baseline();
        alt_baseline.performance_mode = PerformanceMode::Quiet;
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["browser.exe", "heavy.exe"]), || {
                alt_baseline
            }),
            AppProfileTransition::Activate { rule: high }
        );

        // Close heavy app -> falls back to low
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["browser.exe"]), sample_baseline),
            AppProfileTransition::Activate { rule: low }
        );

        // Close browser too (2 misses) -> restores original Balanced baseline, NOT alt_baseline
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&[]), sample_baseline),
            AppProfileTransition::None
        );
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&[]), sample_baseline),
            AppProfileTransition::Restore(sample_baseline())
        );
    }

    #[test]
    fn updates_saved_baseline_on_ac_dc_change_while_active() {
        let mut engine = AppProfileEngine::new();
        let rule = AppProfileRule {
            id: "r1".into(),
            name: "App".into(),
            exe_name: "app.exe".into(),
            performance_mode: PerformanceMode::Performance,
            ..Default::default()
        };
        let cfg = AppProfilesConfig {
            enabled: true,
            poll_interval_secs: 3,
            rules: vec![rule],
        };

        let _ = engine.evaluate(&cfg, &set_of(&["app.exe"]), sample_baseline);
        engine.update_baseline_for_ac_dc(&AutoProfileAction {
            performance_mode: PerformanceMode::Quiet,
            fan_mode: Some(FanMode::CustomCurve),
            curve_preset: Some(FanCurvePreset::Quiet),
            refresh_hz: Some(60),
            dim_keyboard: true,
        });

        let _ = engine.evaluate(&cfg, &set_of(&[]), sample_baseline);
        let restored = engine.evaluate(&cfg, &set_of(&[]), sample_baseline);
        match restored {
            AppProfileTransition::Restore(b) => {
                assert_eq!(b.performance_mode, PerformanceMode::Quiet);
                assert_eq!(b.fan_mode, FanMode::CustomCurve);
                assert_eq!(b.custom_curve, FanCurve::preset_quiet());
                assert_eq!(b.refresh_hz, Some(60));
            }
            other => panic!("expected Restore, got {other:?}"),
        }
    }

    #[test]
    fn disabling_config_restores_immediately_when_active() {
        let mut engine = AppProfileEngine::new();
        let rule = AppProfileRule {
            id: "r1".into(),
            name: "Game".into(),
            exe_name: "game.exe".into(),
            ..Default::default()
        };
        let mut cfg = AppProfilesConfig {
            enabled: true,
            poll_interval_secs: 3,
            rules: vec![rule],
        };

        assert!(matches!(
            engine.evaluate(&cfg, &set_of(&["game.exe"]), sample_baseline),
            AppProfileTransition::Activate { .. }
        ));

        cfg.enabled = false;
        assert_eq!(
            engine.evaluate(&cfg, &set_of(&["game.exe"]), sample_baseline),
            AppProfileTransition::Restore(sample_baseline())
        );
        assert!(engine.active_rule().is_none());
    }
}
