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
