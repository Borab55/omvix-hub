pub mod auto_profile;
pub mod battery_care;
pub mod boards;
pub mod config;
pub mod crash_guard;
pub mod error;
pub mod i18n;
pub mod logging;
pub mod models;
pub mod thermal;
pub mod traits;

pub use config::{AppConfig, BatteryProfile, CustomThemeConfig};
pub use crash_guard::{CrashGuardState, SafeModeReason, crash_guard_path_for};
pub use error::{OmvixError, Result};
pub use i18n::I18n;
pub use logging::{
    clear_logs, default_log_dir, init_logging, list_log_files, tail_logs, total_log_bytes,
    LogConfig, LogHandle, LogLevel, SizeRotatingWriter,
};
pub use models::*;
pub use traits::*;
