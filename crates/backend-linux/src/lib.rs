//! Linux backend scaffold for Omvix Hub.
//!
//! Planned sysfs / kernel / ACPI mappings:
//! - Thermal profile: `/sys/firmware/acpi/platform_profile` & `/sys/firmware/acpi/platform_profile_choices` (`hp-wmi`)
//! - Fan RPM & PWM: `/sys/class/hwmon/hwmon*/fan*_input`, `pwm1`, `pwm1_enable` (`hp-wmi`)
//! - Battery charge limit: `\_SB.WMID.SBCC` via `/proc/acpi/call` (or kernel charge threshold sysfs where supported)
//! - Keyboard RGB: `/sys/class/leds/hp::kbd_backlight` and `/sys/devices/platform/hp-wmi/rgb_zones/*`
//! - Telemetry: `/sys/class/thermal`, `/sys/class/power_supply/BAT*`, NVML

use chrono::Utc;
use omvix_core::boards::lookup_board;
use omvix_core::{
    DiagnosticReport, DiagnosticsBackend, OmvixError, Result, SecurityStatus, SystemIdentity,
};
use std::fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct LinuxDiagnosticsBackend;

impl LinuxDiagnosticsBackend {
    pub fn new() -> Self {
        Self
    }

    fn read_dmi(field: &str) -> String {
        let path = Path::new("/sys/class/dmi/id").join(field);
        fs::read_to_string(path)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }
}

impl DiagnosticsBackend for LinuxDiagnosticsBackend {
    fn collect_diagnostics(&self) -> Result<DiagnosticReport> {
        if !cfg!(target_os = "linux") {
            return Err(OmvixError::UnsupportedHardware(
                "LinuxDiagnosticsBackend is only active on Linux hosts".to_string(),
            ));
        }

        let board_id = Self::read_dmi("board_name");
        let matched_board = lookup_board(&board_id);

        let identity = SystemIdentity {
            manufacturer: Self::read_dmi("sys_vendor"),
            model: Self::read_dmi("product_name"),
            system_family: Self::read_dmi("product_family"),
            board_id,
            bios_version: Self::read_dmi("bios_version"),
            cpu_name: String::new(),
            gpu_names: Vec::new(),
            os_name: std::env::consts::OS.to_string(),
        };

        let hp_wmi_loaded = Path::new("/sys/devices/platform/hp-wmi").exists();
        let acpi_call_present = Path::new("/proc/acpi/call").exists();

        Ok(DiagnosticReport {
            generated_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            identity,
            matched_board,
            security: SecurityStatus {
                is_admin: false,
                vbs_status: None,
                hvci_running: false,
                pawn_io_installed: false,
                undervolt_note: format!(
                    "Linux scaffold: hp-wmi={hp_wmi_loaded}, acpi_call={acpi_call_present}"
                ),
            },
            wmi_hpqbintm_available: hp_wmi_loaded,
            wmi_hpqbevnt_available: false,
            wmi_error: None,
            conflict_services: Vec::new(),
            probes: Vec::new(),
        })
    }
}
