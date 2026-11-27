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

    /// Evaluates the target firmware `BatteryChargeMode` given current charge percentage and AC state.
    /// Automatically clears travel charge once expired, battery reaches >=99%, or AC is unplugged.
    pub fn evaluate(
        &mut self,
        cfg: &mut BatteryConfig,
        charge_pct: u8,
        ac_connected: bool,
    ) -> BatteryChargeMode {
        self.evaluate_at(
            cfg,
            charge_pct,
            ac_connected,
            chrono::Utc::now().timestamp(),
        )
    }

    /// Deterministic version of `evaluate` using caller-supplied `now_utc` Unix seconds.
    pub fn evaluate_at(
        &mut self,
        cfg: &mut BatteryConfig,
        charge_pct: u8,
        ac_connected: bool,
        now_utc: i64,
    ) -> BatteryChargeMode {
        if !cfg.enabled {
            self.last_applied_mode = BatteryChargeMode::Auto;
            return BatteryChargeMode::Auto;
        }

        if cfg.one_time_full_charge || cfg.travel_charge_until_utc.is_some() {
            let expired = cfg
                .travel_charge_until_utc
                .is_some_and(|until| now_utc >= until);
            if expired || charge_pct >= 99 || !ac_connected {
                cfg.clear_travel_charge();
            } else {
                self.last_applied_mode = BatteryChargeMode::Auto;
                return BatteryChargeMode::Auto;
            }
        }

        let stop = cfg.stop_threshold_pct.clamp(50, 100);
        let start = cfg
            .start_threshold_pct
            .clamp(40, stop.saturating_sub(2).max(40));

        let resolve_limit_mode = || match cfg.mode {
            BatteryChargeMode::Hold => BatteryChargeMode::Hold,
            BatteryChargeMode::Inhibit | BatteryChargeMode::Auto | BatteryChargeMode::Unknown => {
                BatteryChargeMode::Inhibit
            }
        };

        let target = if charge_pct >= stop {
            resolve_limit_mode()
        } else if charge_pct <= start {
            BatteryChargeMode::Auto
        } else if self.last_applied_mode != BatteryChargeMode::Auto {
            resolve_limit_mode()
        } else {
            BatteryChargeMode::Auto
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
        cfg.enable_travel_charge(1_800_000_000);
        assert_eq!(
            engine.evaluate_at(&mut cfg, 85, false, 1_800_000_010),
            BatteryChargeMode::Inhibit
        );
        assert!(!cfg.one_time_full_charge);
        assert!(cfg.travel_charge_until_utc.is_none());
    }

    #[test]
    fn travel_charge_expires_after_deadline_and_clears_flag() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            ..BatteryConfig::default()
        };
        let t0 = 1_800_000_000i64;
        cfg.enable_travel_charge(t0);
        let until = cfg.travel_charge_until_utc.unwrap();

        // Before expiry -> Auto
        assert_eq!(
            engine.evaluate_at(&mut cfg, 85, true, until - 1),
            BatteryChargeMode::Auto
        );
        assert!(cfg.one_time_full_charge);

        // At/after expiry -> clears travel mode and reverts to Inhibit
        assert_eq!(
            engine.evaluate_at(&mut cfg, 85, true, until),
            BatteryChargeMode::Inhibit
        );
        assert!(!cfg.one_time_full_charge);
        assert!(cfg.travel_charge_until_utc.is_none());
    }

    #[test]
    fn switching_inhibit_and_hold_inside_hysteresis_band_applies_immediately() {
        let mut engine = BatteryCareEngine::new();
        let mut cfg = BatteryConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            ..BatteryConfig::default()
        };

        assert_eq!(
            engine.evaluate(&mut cfg, 82, true),
            BatteryChargeMode::Inhibit
        );
        cfg.mode = BatteryChargeMode::Hold;
        assert_eq!(engine.evaluate(&mut cfg, 78, true), BatteryChargeMode::Hold);
        cfg.mode = BatteryChargeMode::Inhibit;
        assert_eq!(
            engine.evaluate(&mut cfg, 77, true),
            BatteryChargeMode::Inhibit
        );
    }
}
