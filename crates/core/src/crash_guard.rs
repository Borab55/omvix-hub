use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CRASH_LOOP_WINDOW_SECS: u64 = 60;
pub const CRASH_LOOP_THRESHOLD: usize = 3;
const MAX_STORED_LAUNCHES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafeModeReason {
    CliFlag,
    ShiftHeld,
    CrashLoop,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LaunchRecord {
    pub started_at_unix: u64,
    pub hw_applied_at_unix: Option<u64>,
    pub stabilized_at_unix: Option<u64>,
    pub clean_exit_at_unix: Option<u64>,
}

impl LaunchRecord {
    /// Returns `true` when hardware settings were applied in this session, yet the process
    /// neither survived the 60-second stability window nor exited cleanly.
    pub fn is_post_apply_crash(&self) -> bool {
        self.hw_applied_at_unix.is_some()
            && self.stabilized_at_unix.is_none()
            && self.clean_exit_at_unix.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CrashGuardState {
    pub launches: Vec<LaunchRecord>,
}

pub fn crash_guard_path_for(config_path: &Path) -> PathBuf {
    config_path.with_file_name("crash_guard.json")
}

pub fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn resolve_startup_safe_mode(
    cli_safe: bool,
    shift_held: bool,
    guard: &CrashGuardState,
) -> Option<SafeModeReason> {
    if cli_safe {
        Some(SafeModeReason::CliFlag)
    } else if shift_held {
        Some(SafeModeReason::ShiftHeld)
    } else if guard.is_crash_loop_triggered() {
        Some(SafeModeReason::CrashLoop)
    } else {
        None
    }
}

impl CrashGuardState {
    pub fn load_from_path(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&raw).unwrap_or_default()
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|e| crate::error::OmvixError::Config(e.to_string()))?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, serialized)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Returns `true` when the last 3 launches that applied hardware settings all ended within
    /// 60 seconds without a clean shutdown. Safe-mode launches without hardware writes are ignored.
    pub fn is_crash_loop_triggered(&self) -> bool {
        let recent: Vec<&LaunchRecord> = self
            .launches
            .iter()
            .filter(|r| r.hw_applied_at_unix.is_some())
            .rev()
            .take(CRASH_LOOP_THRESHOLD)
            .collect();
        recent.len() >= CRASH_LOOP_THRESHOLD && recent.iter().all(|r| r.is_post_apply_crash())
    }

    pub fn record_launch_start(&mut self, now_unix: u64) {
        self.launches.push(LaunchRecord {
            started_at_unix: now_unix,
            hw_applied_at_unix: None,
            stabilized_at_unix: None,
            clean_exit_at_unix: None,
        });
        if self.launches.len() > MAX_STORED_LAUNCHES {
            let excess = self.launches.len() - MAX_STORED_LAUNCHES;
            self.launches.drain(0..excess);
        }
    }

    pub fn record_hw_applied(&mut self, now_unix: u64) -> bool {
        let Some(last) = self.launches.last_mut() else {
            return false;
        };
        if last.hw_applied_at_unix.is_none() {
            last.hw_applied_at_unix = Some(now_unix);
            true
        } else {
            false
        }
    }

    pub fn check_and_mark_stabilized(&mut self, now_unix: u64) -> bool {
        let Some(last) = self.launches.last_mut() else {
            return false;
        };
        if let Some(applied) = last.hw_applied_at_unix {
            if last.stabilized_at_unix.is_none()
                && now_unix.saturating_sub(applied) >= CRASH_LOOP_WINDOW_SECS
            {
                last.stabilized_at_unix = Some(now_unix);
                return true;
            }
        }
        false
    }

    pub fn mark_clean_exit(&mut self, now_unix: u64) {
        if let Some(last) = self.launches.last_mut() {
            last.clean_exit_at_unix = Some(now_unix);
        }
    }

    pub fn clear_history(&mut self) {
        self.launches.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_loop_triggers_after_three_post_apply_crashes_and_ignores_safe_launches() {
        let mut g = CrashGuardState::default();

        // 2 post-apply crashes -> not yet triggered
        g.record_launch_start(100);
        assert!(g.record_hw_applied(102));
        assert!(!g.is_crash_loop_triggered());

        g.record_launch_start(200);
        assert!(g.record_hw_applied(202));
        assert!(!g.is_crash_loop_triggered());

        // Safe launch with zero hardware writes -> ignored cleanly
        g.record_launch_start(250);
        assert!(!g.is_crash_loop_triggered());

        // 3rd post-apply crash -> triggers CrashLoop
        g.record_launch_start(300);
        assert!(g.record_hw_applied(302));
        assert!(g.is_crash_loop_triggered());

        // Subsequent safe launch without applying hardware must NOT erase the crash-loop state
        g.record_launch_start(350);
        assert!(g.is_crash_loop_triggered());

        // A launch that stabilizes (>= 60s) breaks the crash-loop chain
        let mut g_stab = g.clone();
        g_stab.record_launch_start(400);
        assert!(g_stab.record_hw_applied(402));
        assert!(g_stab.check_and_mark_stabilized(462));
        assert!(!g_stab.is_crash_loop_triggered());

        // A clean exit also breaks the crash-loop chain
        let mut g_clean = g;
        g_clean.record_launch_start(500);
        assert!(g_clean.record_hw_applied(502));
        g_clean.mark_clean_exit(510);
        assert!(!g_clean.is_crash_loop_triggered());
    }

    #[test]
    fn stabilization_requires_full_60_second_window() {
        let mut g = CrashGuardState::default();
        for t in [100u64, 200] {
            g.record_launch_start(t);
            assert!(g.record_hw_applied(t + 2));
        }

        g.record_launch_start(300);
        assert!(g.record_hw_applied(305));
        assert!(!g.check_and_mark_stabilized(364)); // +59s -> not stabilized yet
        assert!(g.is_crash_loop_triggered());

        assert!(g.check_and_mark_stabilized(365)); // +60s -> stabilized
        assert!(!g.check_and_mark_stabilized(380)); // idempotent
        assert!(!g.is_crash_loop_triggered());
    }

    #[test]
    fn resolve_startup_safe_mode_priority_and_persistence_roundtrip() {
        let mut crashed = CrashGuardState::default();
        for t in [100u64, 200, 300] {
            crashed.record_launch_start(t);
            crashed.record_hw_applied(t + 1);
        }

        assert_eq!(
            resolve_startup_safe_mode(true, true, &crashed),
            Some(SafeModeReason::CliFlag)
        );
        assert_eq!(
            resolve_startup_safe_mode(false, true, &crashed),
            Some(SafeModeReason::ShiftHeld)
        );
        assert_eq!(
            resolve_startup_safe_mode(false, false, &crashed),
            Some(SafeModeReason::CrashLoop)
        );
        assert_eq!(
            resolve_startup_safe_mode(false, false, &CrashGuardState::default()),
            None
        );

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_crash_guard_test_{nanos}"));
        let cfg_path = dir.join("config.toml");
        let guard_path = crash_guard_path_for(&cfg_path);
        assert_eq!(guard_path, dir.join("crash_guard.json"));

        crashed.save_to_path(&guard_path).expect("save crash guard");
        assert!(!guard_path.with_extension("json.tmp").exists());
        let loaded = CrashGuardState::load_from_path(&guard_path);
        assert_eq!(loaded, crashed);
        assert!(loaded.is_crash_loop_triggered());

        let _ = fs::remove_dir_all(&dir);
    }
}
