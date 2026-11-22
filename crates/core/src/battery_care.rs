use crate::config::BatteryConfig;
use crate::models::BatteryChargeMode;

/// Stateful hysteresis engine for HP battery charge limiting and one-time 100% travel mode.
#[derive(Debug, Clone, Default)]
pub struct BatteryCareEngine {
    last_applied_mode: BatteryChargeMode,
}

impl BatteryCareEngine {
    pub fn new() -> Self {
        Self {
            last_applied_mode: BatteryChargeMode::Auto,
        }
    }

    pub fn last_mode(&self) -> BatteryChargeMode {
        self.last_applied_mode
    }

    /// Evaluates the target firmware `BatteryChargeMode` given current charge percentage and AC state.
    /// Automatically clears `cfg.one_time_full_charge` once battery reaches >=99% or AC is unplugged.
    pub fn evaluate(
        &mut self,
        cfg: &mut BatteryConfig,
        charge_pct: u8,
        ac_connected: bool,
    ) -> BatteryChargeMode {
        if !cfg.enabled {
            self.last_applied_mode = BatteryChargeMode::Auto;
            return BatteryChargeMode::Auto;
        }

        if cfg.one_time_full_charge {
            if charge_pct >= 99 || !ac_connected {
                cfg.one_time_full_charge = false;
            } else {
                self.last_applied_mode = BatteryChargeMode::Auto;
                return BatteryChargeMode::Auto;
            }
        }

        let stop = cfg.stop_threshold_pct.clamp(50, 100);
        let start = cfg.start_threshold_pct.clamp(40, stop.saturating_sub(2));

        let target = if charge_pct >= stop {
            match cfg.mode {
                BatteryChargeMode::Hold => BatteryChargeMode::Hold,
                BatteryChargeMode::Inhibit | BatteryChargeMode::Auto => BatteryChargeMode::Inhibit,
            }
        } else if charge_pct <= start {
            BatteryChargeMode::Auto
        } else {
            self.last_applied_mode
        };

        self.last_applied_mode = target;
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_always_returns_auto() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(engine.evaluate(&mut cfg, 90, true), BatteryChargeMode::Auto);
    }

    #[test]
    fn threshold_stop_hysteresis_and_resume_work() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: false,
            one_time_full_charge: false,
            ..BatteryConfig::default()
        };

        // Below start -> Auto
        assert_eq!(engine.evaluate(&mut cfg, 70, true), BatteryChargeMode::Auto);
        // Between start and stop -> preserves Auto
        assert_eq!(engine.evaluate(&mut cfg, 78, true), BatteryChargeMode::Auto);
        // At stop -> Inhibit
        assert_eq!(
            engine.evaluate(&mut cfg, 80, true),
            BatteryChargeMode::Inhibit
        );
        // Drop to 77 (still above 75) -> preserves Inhibit
        assert_eq!(
            engine.evaluate(&mut cfg, 77, true),
            BatteryChargeMode::Inhibit
        );
        // Drop to 75 -> resumes Auto
        assert_eq!(engine.evaluate(&mut cfg, 75, true), BatteryChargeMode::Auto);
    }

    #[test]
    fn hold_mode_selected_when_configured() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: true,
            mode: BatteryChargeMode::Hold,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: true,
            one_time_full_charge: false,
            ..BatteryConfig::default()
        };
        assert_eq!(engine.evaluate(&mut cfg, 85, true), BatteryChargeMode::Hold);
    }

    #[test]
    fn one_time_travel_charge_reverts_at_full_or_unplug() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: false,
            one_time_full_charge: true,
            ..BatteryConfig::default()
        };

        // Even at 85%, travel mode keeps Auto until 99% or unplug
        assert_eq!(engine.evaluate(&mut cfg, 85, true), BatteryChargeMode::Auto);
        assert!(cfg.one_time_full_charge);

        // At 99%, travel mode finishes and reverts to Inhibit
        assert_eq!(
            engine.evaluate(&mut cfg, 99, true),
            BatteryChargeMode::Inhibit
        );
        assert!(!cfg.one_time_full_charge);

        // Unplugging also cancels travel mode
        cfg.one_time_full_charge = true;
        assert_eq!(
            engine.evaluate(&mut cfg, 85, false),
            BatteryChargeMode::Inhibit
        );
        assert!(!cfg.one_time_full_charge);
    }
}
