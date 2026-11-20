#[cfg(windows)]
mod system_info;
#[cfg(windows)]
pub mod wmi_bios;
#[cfg(windows)]
mod wmi_com;

use chrono::Utc;
use omvix_core::boards::lookup_board;
use omvix_core::{DiagnosticReport, DiagnosticsBackend, Result};
#[cfg(not(windows))]
use omvix_core::OmvixError;

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
