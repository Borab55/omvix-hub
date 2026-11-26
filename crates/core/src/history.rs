use crate::models::{
    BatteryChargeMode, CpuBoostMode, FanCurve, FanMode, PerformanceMode, RgbColor,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_CHANGE_HISTORY: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeDomain {
    PerformanceMode,
    FanMode,
    BatteryLimit,
    DisplayRefreshRate,
    CpuBoost,
    KeyboardLighting,
    PowerLimits,
}

impl ChangeDomain {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::PerformanceMode => "history.domain_performance",
            Self::FanMode => "history.domain_fan",
            Self::BatteryLimit => "history.domain_battery",
            Self::DisplayRefreshRate => "history.domain_display",
            Self::CpuBoost => "history.domain_cpu_boost",
            Self::KeyboardLighting => "history.domain_lighting",
            Self::PowerLimits => "history.domain_power_limits",
        }
    }

    pub fn requires_bios_write(self) -> bool {
        !matches!(self, Self::DisplayRefreshRate | Self::CpuBoost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertPayload {
    PerformanceMode(PerformanceMode),
    FanMode {
        mode: FanMode,
        gpu_mode: FanMode,
        manual_speed_pct: u8,
        gpu_manual_speed_pct: u8,
        custom_curve: FanCurve,
        gpu_custom_curve: FanCurve,
    },
    BatteryLimit {
        enabled: bool,
        mode: BatteryChargeMode,
        stop_threshold_pct: u8,
        start_threshold_pct: u8,
        hold_when_shutdown: bool,
        one_time_full_charge: bool,
        travel_charge_until_utc: Option<i64>,
        active_profile_id: Option<String>,
    },
    DisplayRefreshRate(u32),
    CpuBoost(CpuBoostMode),
    KeyboardLighting {
        enabled: bool,
        brightness_pct: u8,
        zones: [RgbColor; 4],
    },
    PowerLimits {
        pl1_watts: Option<u16>,
        pl2_watts: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareChangeEntry {
    pub id: u64,
    pub timestamp: DateTime<Utc>,
    pub domain: ChangeDomain,
    pub old_summary: String,
    pub new_summary: String,
    pub revert: RevertPayload,
}

/// Fixed-capacity ring buffer tracking recent interactive hardware changes for 1-click Undo.
#[derive(Debug, Clone, Default)]
pub struct ChangeHistory {
    entries: VecDeque<HardwareChangeEntry>,
    next_id: u64,
}

impl ChangeHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(
        &mut self,
        domain: ChangeDomain,
        old_summary: impl Into<String>,
        new_summary: impl Into<String>,
        revert: RevertPayload,
    ) -> Option<u64> {
        let old_summary = old_summary.into();
        let new_summary = new_summary.into();
        if old_summary == new_summary {
            return None;
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.entries.push_front(HardwareChangeEntry {
            id,
            timestamp: Utc::now(),
            domain,
            old_summary,
            new_summary,
            revert,
        });
        while self.entries.len() > MAX_CHANGE_HISTORY {
            self.entries.pop_back();
        }
        Some(id)
    }

    pub fn entries_vec(&self) -> Vec<HardwareChangeEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn latest(&self) -> Option<&HardwareChangeEntry> {
        self.entries.front()
    }

    pub fn peek_by_id(&self, id: u64) -> Option<&HardwareChangeEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn pop_latest(&mut self) -> Option<HardwareChangeEntry> {
        self.entries.pop_front()
    }

    pub fn remove_by_id(&mut self, id: u64) -> Option<HardwareChangeEntry> {
        let pos = self.entries.iter().position(|e| e.id == id)?;
        self.entries.remove(pos)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_at_ten_and_evicts_oldest() {
        let mut hist = ChangeHistory::new();
        for i in 1..=14u8 {
            let id = hist.record(
                ChangeDomain::FanMode,
                format!("Manual {}%", i - 1),
                format!("Manual {i}%"),
                RevertPayload::FanMode {
                    mode: FanMode::Manual,
                    gpu_mode: FanMode::Auto,
                    manual_speed_pct: i - 1,
                    gpu_manual_speed_pct: i - 1,
                    custom_curve: FanCurve::default(),
                    gpu_custom_curve: FanCurve::default(),
                },
            );
            assert_eq!(id, Some(u64::from(i)));
        }
        assert_eq!(hist.len(), MAX_CHANGE_HISTORY);
        let list = hist.entries_vec();
        assert_eq!(list.first().unwrap().id, 14);
        assert_eq!(list.last().unwrap().id, 5);
        assert!(hist.peek_by_id(4).is_none());
        assert!(hist.peek_by_id(5).is_some());
    }

    #[test]
    fn skips_identical_change_and_supports_pop_remove_clear() {
        let mut hist = ChangeHistory::new();
        assert_eq!(
            hist.record(
                ChangeDomain::PerformanceMode,
                "Balanced",
                "Balanced",
                RevertPayload::PerformanceMode(PerformanceMode::Balanced),
            ),
            None
        );
        assert!(hist.is_empty());

        let id1 = hist
            .record(
                ChangeDomain::PerformanceMode,
                "Balanced",
                "Eco",
                RevertPayload::PerformanceMode(PerformanceMode::Balanced),
            )
            .unwrap();
        let id2 = hist
            .record(
                ChangeDomain::DisplayRefreshRate,
                "165 Hz",
                "60 Hz",
                RevertPayload::DisplayRefreshRate(165),
            )
            .unwrap();
        let _id3 = hist
            .record(
                ChangeDomain::CpuBoost,
                "Aggressive",
                "Disabled",
                RevertPayload::CpuBoost(CpuBoostMode::Aggressive),
            )
            .unwrap();

        assert_eq!(hist.latest().unwrap().domain, ChangeDomain::CpuBoost);
        assert_eq!(hist.pop_latest().unwrap().domain, ChangeDomain::CpuBoost);
        assert_eq!(hist.remove_by_id(id1).unwrap().id, id1);
        assert_eq!(hist.len(), 1);
        assert_eq!(hist.latest().unwrap().id, id2);
        hist.clear();
        assert!(hist.is_empty());
    }

    #[test]
    fn bios_write_flag_matches_domain_semantics() {
        assert!(!ChangeDomain::DisplayRefreshRate.requires_bios_write());
        assert!(!ChangeDomain::CpuBoost.requires_bios_write());
        assert!(ChangeDomain::PerformanceMode.requires_bios_write());
        assert!(ChangeDomain::FanMode.requires_bios_write());
        assert!(ChangeDomain::BatteryLimit.requires_bios_write());
        assert!(ChangeDomain::KeyboardLighting.requires_bios_write());
        assert!(ChangeDomain::PowerLimits.requires_bios_write());
    }
}
