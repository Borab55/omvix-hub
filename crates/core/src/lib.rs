pub mod app_profiles;
pub mod auto_profile;
pub mod battery_care;
pub mod battery_history;
pub mod boards;
pub mod config;
pub mod crash_guard;
pub mod error;
pub mod history;
pub mod i18n;
pub mod logging;
pub mod models;
pub mod support_bundle;
pub mod telemetry_csv;
pub mod thermal;
pub mod traits;
pub mod updater;

pub use app_profiles::{AppProfileRule, AppProfilesConfig};
pub use battery_history::{BatteryHealthHistory, BatteryHealthRecord};
pub use config::{
    AppConfig, BatteryProfile, ConfigLoadOutcome, CustomThemeConfig, LightingProfile,
};
pub use crash_guard::{crash_guard_path_for, CrashGuardState, SafeModeReason};
pub use error::{OmvixError, Result};
pub use history::{ChangeDomain, ChangeHistory, HardwareChangeEntry, RevertPayload};
pub use i18n::I18n;
pub use logging::{
    clear_logs, default_log_dir, init_logging, list_log_files, tail_logs, total_log_bytes,
    LogConfig, LogHandle, LogLevel, SizeRotatingWriter,
};
pub use models::*;
pub use telemetry_csv::TelemetryCsvRow;
pub use traits::*;
pub use updater::{
    AuthenticodeInfo, ReleaseAsset, ReleaseInfo, SemVer, StagedUpdate, UpdateSignatureStatus,
    UpdateUiStatus,
};

pub(crate) fn write_bytes_atomically(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_ext = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{ext}.tmp"),
        _ => "tmp".to_string(),
    };
    let tmp_path = path.with_extension(tmp_ext);
    std::fs::write(&tmp_path, bytes)?;
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err.into());
    }
    Ok(())
}
