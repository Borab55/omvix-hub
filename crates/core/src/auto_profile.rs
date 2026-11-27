use crate::config::AutoProfileConfig;
use crate::models::{FanCurvePreset, FanMode, PerformanceMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoProfileAction {
    pub performance_mode: PerformanceMode,
    pub fan_mode: Option<FanMode>,
    pub curve_preset: Option<FanCurvePreset>,
    pub refresh_hz: Option<u32>,
    pub dim_keyboard: bool,
}

/// Tracks AC/DC transitions and determines when an automatic power/display/keyboard profile change should fire.
#[derive(Debug, Clone, Default)]
pub struct AutoProfileEngine {
    last_ac_state: Option<bool>,
}

impl AutoProfileEngine {
    pub fn new() -> Self {
        Self {
            last_ac_state: None,
        }
    }

    /// Evaluates whether an AC/DC transition occurred (or `force == true`).
    /// Returns `None` if auto-profile is disabled or power state did not change.
    pub fn evaluate(
        &mut self,
        cfg: &AutoProfileConfig,
        ac_connected: bool,
        force: bool,
    ) -> Option<AutoProfileAction> {
        if !cfg.enabled {
            self.last_ac_state = Some(ac_connected);
            return None;
        }

        let changed = self.last_ac_state != Some(ac_connected);
        self.last_ac_state = Some(ac_connected);

        if !changed && !force {
            return None;
        }

        if ac_connected {
            Some(AutoProfileAction {
                performance_mode: cfg.ac_performance_mode,
                fan_mode: cfg.ac_fan_mode,
                curve_preset: cfg.ac_curve_preset,
                refresh_hz: cfg.ac_refresh_hz,
                dim_keyboard: false,
            })
        } else {
            Some(AutoProfileAction {
                performance_mode: cfg.dc_performance_mode,
                fan_mode: cfg.dc_fan_mode,
                curve_preset: cfg.dc_curve_preset,
                refresh_hz: cfg.dc_refresh_hz,
                dim_keyboard: cfg.dim_keyboard_on_battery,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_only_on_transition_unless_forced() {
        let mut engine = AutoProfileEngine::new();
        let cfg = AutoProfileConfig {
            enabled: true,
            ac_performance_mode: PerformanceMode::Performance,
            dc_performance_mode: PerformanceMode::Quiet,
            ac_fan_mode: Some(FanMode::CustomCurve),
            dc_fan_mode: Some(FanMode::Auto),
            ac_curve_preset: Some(FanCurvePreset::Aggressive),
            dc_curve_preset: None,
            ac_refresh_hz: Some(165),
            dc_refresh_hz: Some(60),
            dim_keyboard_on_battery: true,
        };

        // Initial AC observation triggers action
        let ac_action = engine
            .evaluate(&cfg, true, false)
            .expect("initial AC action");
        assert_eq!(ac_action.performance_mode, PerformanceMode::Performance);
        assert_eq!(ac_action.fan_mode, Some(FanMode::CustomCurve));
        assert_eq!(ac_action.curve_preset, Some(FanCurvePreset::Aggressive));
        assert_eq!(ac_action.refresh_hz, Some(165));
        assert!(!ac_action.dim_keyboard);

        // Same AC state -> None
        assert!(engine.evaluate(&cfg, true, false).is_none());

        // Transition to battery -> DC action
        let dc_action = engine.evaluate(&cfg, false, false).expect("DC transition");
        assert_eq!(dc_action.performance_mode, PerformanceMode::Quiet);
        assert_eq!(dc_action.fan_mode, Some(FanMode::Auto));
        assert_eq!(dc_action.curve_preset, None);
        assert_eq!(dc_action.refresh_hz, Some(60));
        assert!(dc_action.dim_keyboard);

        // Forced evaluation returns current DC action
        assert!(engine.evaluate(&cfg, false, true).is_some());
    }

    #[test]
    fn disabled_auto_profile_returns_none() {
        let mut engine = AutoProfileEngine::new();
        let cfg = AutoProfileConfig::default(); // enabled = false
        assert!(engine.evaluate(&cfg, true, true).is_none());
    }
}
