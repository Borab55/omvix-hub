use crate::error::{OmvixError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_BATTERY_HISTORY_ENTRIES: usize = 104;
pub const WEEKLY_INTERVAL_SECS: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryHealthRecord {
    pub recorded_at_unix: i64,
    pub date_label: String,
    pub design_capacity_mwh: u32,
    pub full_charge_capacity_mwh: u32,
    pub health_pct: f32,
    pub wear_pct: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryHealthHistory {
    pub records: Vec<BatteryHealthRecord>,
}

pub fn battery_history_path_for(anchor_path: &Path) -> PathBuf {
    anchor_path.with_file_name("battery_health_history.json")
}

fn build_record(
    design_mwh: u32,
    full_mwh: u32,
    now_unix: i64,
    date_label: String,
) -> Option<BatteryHealthRecord> {
    if design_mwh == 0 || full_mwh == 0 || full_mwh > design_mwh.saturating_mul(2) {
        return None;
    }
    let health_pct = ((full_mwh as f32 / design_mwh as f32) * 100.0).clamp(0.0, 100.0);
    let wear_pct = (100.0 - health_pct).clamp(0.0, 100.0);
    Some(BatteryHealthRecord {
        recorded_at_unix: now_unix,
        date_label,
        design_capacity_mwh: design_mwh,
        full_charge_capacity_mwh: full_mwh,
        health_pct,
        wear_pct,
    })
}

impl BatteryHealthHistory {
    pub fn load_from_path(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| OmvixError::Config(format!("Serialize battery history failed: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Automatically logs battery health on first run, after 7 days, or after >= 24h when
    /// reported full capacity shifts significantly (`>= max(design/50, 500) mWh`).
    pub fn record_if_due(
        &mut self,
        design_mwh: u32,
        full_mwh: u32,
        now_unix: i64,
        date_label: String,
    ) -> bool {
        let Some(rec) = build_record(design_mwh, full_mwh, now_unix, date_label) else {
            return false;
        };

        let due = match self.records.last() {
            None => true,
            Some(last) => {
                let elapsed = now_unix.saturating_sub(last.recorded_at_unix);
                let delta = full_mwh.abs_diff(last.full_charge_capacity_mwh);
                let significant = delta >= (design_mwh / 50).max(500);
                elapsed >= WEEKLY_INTERVAL_SECS || (elapsed >= 24 * 3600 && significant)
            }
        };

        if !due {
            return false;
        }

        self.records.push(rec);
        while self.records.len() > MAX_BATTERY_HISTORY_ENTRIES {
            self.records.remove(0);
        }
        true
    }

    /// Manually logs (or updates today's entry) regardless of the weekly interval.
    pub fn force_record_snapshot(
        &mut self,
        design_mwh: u32,
        full_mwh: u32,
        now_unix: i64,
        date_label: String,
    ) -> bool {
        let Some(rec) = build_record(design_mwh, full_mwh, now_unix, date_label) else {
            return false;
        };
        if let Some(last) = self.records.last_mut() {
            if last.date_label == rec.date_label {
                *last = rec;
                return true;
            }
        }
        self.records.push(rec);
        while self.records.len() > MAX_BATTERY_HISTORY_ENTRIES {
            self.records.remove(0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_initially_deduplicates_weekly_and_detects_significant_shift() {
        let mut hist = BatteryHealthHistory::default();

        // Invalid readings rejected
        assert!(!hist.record_if_due(0, 50_000, 1_000, "2026-11-01".into()));
        assert!(!hist.record_if_due(70_000, 0, 1_000, "2026-11-01".into()));
        assert!(!hist.record_if_due(70_000, 200_000, 1_000, "2026-11-01".into()));

        // Initial record
        assert!(hist.record_if_due(70_000, 66_500, 1_000, "2026-11-01".into()));
        assert_eq!(hist.records.len(), 1);
        assert!((hist.records[0].health_pct - 95.0).abs() < 0.01);
        assert!((hist.records[0].wear_pct - 5.0).abs() < 0.01);

        // Same day / < 7 days -> skipped
        assert!(!hist.record_if_due(70_000, 66_400, 1_000 + 3 * 86_400, "2026-11-04".into()));
        assert_eq!(hist.records.len(), 1);

        // >= 24h with >= 2% delta (>= 1400 mWh) -> recorded early
        assert!(hist.record_if_due(70_000, 64_000, 1_000 + 25 * 3_600, "2026-11-02".into()));
        assert_eq!(hist.records.len(), 2);

        // >= 7 days -> weekly record
        assert!(hist.record_if_due(
            70_000,
            63_900,
            1_000 + 25 * 3_600 + WEEKLY_INTERVAL_SECS,
            "2026-11-09".into()
        ));
        assert_eq!(hist.records.len(), 3);
    }

    #[test]
    fn force_snapshot_updates_same_day_and_caps_at_max() {
        let mut hist = BatteryHealthHistory::default();
        assert!(hist.force_record_snapshot(70_000, 65_000, 100, "2026-11-22".into()));
        assert!(hist.force_record_snapshot(70_000, 64_500, 200, "2026-11-22".into()));
        assert_eq!(hist.records.len(), 1);
        assert_eq!(hist.records[0].full_charge_capacity_mwh, 64_500);

        for i in 0..120 {
            assert!(hist.force_record_snapshot(
                70_000,
                60_000,
                1_000 + i * 86_400,
                format!("day-{i}")
            ));
        }
        assert_eq!(hist.records.len(), MAX_BATTERY_HISTORY_ENTRIES);
        assert_eq!(hist.records.last().unwrap().date_label, "day-119");
    }

    #[test]
    fn persistence_roundtrip_writes_atomically() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_bh_test_{nanos}"));
        let anchor = dir.join("config.toml");
        let path = battery_history_path_for(&anchor);
        assert_eq!(path, dir.join("battery_health_history.json"));

        let mut hist = BatteryHealthHistory::default();
        hist.force_record_snapshot(70_000, 66_000, 1_760_000_000, "2026-11-22".into());
        hist.save_to_path(&path).expect("save");
        assert!(!path.with_extension("json.tmp").exists());

        let loaded = BatteryHealthHistory::load_from_path(&path);
        assert_eq!(loaded, hist);

        let _ = fs::remove_dir_all(&dir);
    }
}
