use thiserror::Error;

/// Unified error type across Omvix Hub core and platform backends.
#[derive(Debug, Error)]
pub enum OmvixError {
    #[error("HP WMI BIOS interface (root\\WMI\\hpqBIntM) is unavailable: {0}")]
    WmiUnavailable(String),

    #[error(
        "HP WMI BIOS call failed (cmd=0x{command:X}, type=0x{command_type:X}, rc={return_code}): {message}"
    )]
    WmiCallFailed {
        command: u32,
        command_type: u32,
        return_code: u32,
        message: String,
    },

    #[error("Administrator / elevated privileges are required: {0}")]
    PermissionDenied(String),

    #[error("Hardware or firmware does not support this feature: {0}")]
    UnsupportedHardware(String),

    #[error("Operation blocked by Windows VBS / Memory Integrity (HVCI): {0}")]
    VbsLocked(String),

    #[error("Optional kernel driver is required for this feature: {0}")]
    DriverRequired(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, OmvixError>;
