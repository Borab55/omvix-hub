#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod battery;
#[cfg(windows)]
pub mod battery_service;
#[cfg(windows)]
pub mod display;
#[cfg(windows)]
pub mod emi;
#[cfg(windows)]
pub mod gpu_mux;
#[cfg(windows)]
pub mod hotkeys;
#[cfg(windows)]
pub mod keyboard;
#[cfg(windows)]
pub mod notifications;
#[cfg(windows)]
pub mod paths;
#[cfg(windows)]
pub mod pawnio;
#[cfg(windows)]
pub mod power_events;
#[cfg(windows)]
pub mod power_mode;
#[cfg(windows)]
pub mod processes;
#[cfg(windows)]
pub mod security_fs;
#[cfg(windows)]
pub mod system_info;
#[cfg(windows)]
pub mod telemetry;
#[cfg(windows)]
pub mod thermal_fan;
#[cfg(windows)]
pub mod tuning;
#[cfg(windows)]
pub mod unelevated;
#[cfg(windows)]
pub mod updater;
#[cfg(windows)]
pub mod wmi_bios;
#[cfg(windows)]
mod wmi_com;

use chrono::Utc;
use omvix_core::boards::lookup_board;
use omvix_core::{DiagnosticReport, DiagnosticsBackend, Result};
#[cfg(not(windows))]
use omvix_core::OmvixError;

#[cfg(windows)]
pub use battery::WindowsBatteryBackend;
#[cfg(windows)]
pub use display::WindowsDisplayBackend;
#[cfg(windows)]
pub use gpu_mux::WindowsGpuSwitchBackend;
#[cfg(windows)]
pub use keyboard::WindowsKeyboardBackend;
#[cfg(windows)]
pub use telemetry::WindowsTelemetryBackend;
#[cfg(windows)]
pub use thermal_fan::WindowsThermalFanBackend;
#[cfg(windows)]
pub use tuning::WindowsCpuTuningBackend;

#[derive(Debug, Default)]
pub struct WindowsDiagnosticsBackend;

impl WindowsDiagnosticsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl DiagnosticsBackend for WindowsDiagnosticsBackend {
    fn collect_diagnostics(&self) -> Result<DiagnosticReport> {
        #[cfg(not(windows))]
        {
            Err(OmvixError::UnsupportedHardware(
                "WindowsDiagnosticsBackend is only supported on Windows".to_string(),
            ))
        }

        #[cfg(windows)]
        {
            let identity = system_info::collect_system_identity()?;
            let matched_board = lookup_board(&identity.board_id);
            let security = system_info::collect_security_status();
            let (hpqbintm_class_exists, hpqbevnt_class_exists) =
                system_info::check_hp_wmi_classes();
            let conflict_services = system_info::scan_conflict_services();

            let (wmi_hpqbintm_available, wmi_error, probes) = match wmi_bios::HpWmiBios::connect()
            {
                Ok(bios) => (true, None, bios.probe_all_readonly()),
                Err(err) => {
                    tracing::warn!("HP WMI BIOS probe skipped: {err}");
                    (hpqbintm_class_exists, Some(err.to_string()), Vec::new())
                }
            };

            Ok(DiagnosticReport {
                generated_at: Utc::now(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                identity,
                matched_board,
                security,
                wmi_hpqbintm_available,
                wmi_hpqbevnt_available: hpqbevnt_class_exists,
                wmi_error,
                conflict_services,
                probes,
            })
        }
    }
}
