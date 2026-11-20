use crate::system_info::collect_security_status;
use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code};
use omvix_core::{CpuBoostMode, CpuTuningBackend, OmvixError, Result};
use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadACValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex,
    PowerWriteDCValueIndex,
};
use windows::Win32::System::Registry::HKEY;
use windows::core::GUID;

// Processor power settings subgroup GUID: 54533251-82be-4824-96c1-47b60b740d00
const SUB_PROCESSOR: GUID = GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00);
// Processor performance boost mode GUID: be337238-0d82-4146-a960-4f3749d470c7
const PERFBOOSTMODE: GUID = GUID::from_u128(0xbe337238_0d82_4146_a960_4f3749d470c7);

/// Windows CPU Boost (Power Plan API), Power Limits (WMI/MSR), TCC & Undervolt backend.
#[derive(Debug, Default)]
pub struct WindowsCpuTuningBackend;

impl WindowsCpuTuningBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn read_cpu_boost_mode(&self) -> Result<CpuBoostMode> {
        unsafe {
            let mut active_ptr: *mut GUID = std::ptr::null_mut();
            let err = PowerGetActiveScheme(HKEY::default(), &mut active_ptr);
            if err != ERROR_SUCCESS || active_ptr.is_null() {
                return Err(OmvixError::WmiUnavailable(format!(
                    "PowerGetActiveScheme failed: {}",
                    err.0
                )));
            }
            let scheme = *active_ptr;
            let _ = LocalFree(HLOCAL(active_ptr as _));

            let mut val = 0u32;
            let read_err = PowerReadACValueIndex(
                HKEY::default(),
                Some(&scheme),
                Some(&SUB_PROCESSOR),
                Some(&PERFBOOSTMODE),
                &mut val,
            );
            if read_err != ERROR_SUCCESS {
                return Err(OmvixError::UnsupportedHardware(format!(
                    "PowerReadACValueIndex(PERFBOOSTMODE) failed: {}",
                    read_err.0
                )));
            }

            Ok(match val {
                0 => CpuBoostMode::Disabled,
                1 => CpuBoostMode::Enabled,
                2 => CpuBoostMode::Aggressive,
                4 => CpuBoostMode::EfficientAggressive,
                _ => CpuBoostMode::Aggressive,
            })
        }
    }

    fn verify_msr_unlocked(&self) -> Result<()> {
        let sec = collect_security_status();
        if sec.hvci_running || sec.vbs_status == Some(2) {
            return Err(OmvixError::VbsLocked(
                "Windows VBS / Memory Integrity (HVCI) is active and blocks ring-0 MSR writes."
                    .to_string(),
            ));
        }
        if !sec.pawn_io_installed {
            return Err(OmvixError::DriverRequired(
                "PawnIO driver is required for direct MSR tuning (TCC / Undervolt / MSR PL1-PL2)."
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl CpuTuningBackend for WindowsCpuTuningBackend {
    fn set_cpu_boost_mode(&self, mode: CpuBoostMode) -> Result<()> {
        let idx = match mode {
            CpuBoostMode::Disabled => 0u32,
            CpuBoostMode::Enabled => 1u32,
            CpuBoostMode::Aggressive => 2u32,
            CpuBoostMode::EfficientAggressive => 4u32,
        };

        unsafe {
            let mut active_ptr: *mut GUID = std::ptr::null_mut();
            let err = PowerGetActiveScheme(HKEY::default(), &mut active_ptr);
            if err != ERROR_SUCCESS || active_ptr.is_null() {
                return Err(OmvixError::WmiUnavailable(format!(
                    "PowerGetActiveScheme failed: {}",
                    err.0
                )));
            }
            let scheme = *active_ptr;
            let _ = LocalFree(HLOCAL(active_ptr as _));

            let ac_err = PowerWriteACValueIndex(
                HKEY::default(),
                &scheme,
                Some(&SUB_PROCESSOR),
                Some(&PERFBOOSTMODE),
                idx,
            );
            if ac_err != ERROR_SUCCESS {
                return Err(OmvixError::PermissionDenied(format!(
                    "PowerWriteACValueIndex failed ({})",
                    ac_err.0
                )));
            }

            let dc_rc = PowerWriteDCValueIndex(
                HKEY::default(),
                &scheme,
                Some(&SUB_PROCESSOR),
                Some(&PERFBOOSTMODE),
                idx,
            );
            if dc_rc != ERROR_SUCCESS.0 {
                return Err(OmvixError::PermissionDenied(format!(
                    "PowerWriteDCValueIndex failed ({dc_rc})"
                )));
            }

            let apply_err = PowerSetActiveScheme(HKEY::default(), Some(&scheme));
            if apply_err != ERROR_SUCCESS {
                return Err(OmvixError::WmiUnavailable(format!(
                    "PowerSetActiveScheme failed ({})",
                    apply_err.0
                )));
            }
        }

        tracing::info!(?mode, idx, "Updated Windows CPU Turbo Boost mode");
        Ok(())
    }

    fn set_power_limits(&self, pl1_watts: Option<u16>, pl2_watts: Option<u16>) -> Result<()> {
        let pl1 = pl1_watts.map(|w| w.clamp(15, 200)).unwrap_or(55);
        let pl2 = pl2_watts.map(|w| w.clamp(15, 200)).unwrap_or(115);

        if let Ok(bios) = HpWmiBios::connect() {
            let payload = [pl1 as u8, pl2 as u8, 0xFF, 0xFF];
            let resp = bios.execute(0x20008, 0x29, &payload, OutputSize::Size0)?;
            if resp.return_code == 0 {
                tracing::info!(pl1, pl2, "Applied CPU/Platform power limits via HP WMI 0x29");
                return Ok(());
            }
            tracing::warn!(
                rc = resp.return_code,
                msg = %describe_wmi_return_code(resp.return_code),
                "HP WMI 0x29 power limit command rejected; checking MSR fallback"
            );
        }

        self.verify_msr_unlocked()?;
        Err(OmvixError::UnsupportedHardware(
            "PawnIO MSR 0x610 power-limit bridge requires explicit PawnIO module initialization"
                .to_string(),
        ))
    }

    fn set_tcc_temp_limit(&self, temp_c: u8) -> Result<()> {
        let clamped = temp_c.clamp(70, 100);
        self.verify_msr_unlocked()?;
        Err(OmvixError::UnsupportedHardware(format!(
            "TCC temperature limit ({clamped}°C, MSR 0x1A2) requires PawnIO MSR module bridge"
        )))
    }

    fn set_core_undervolt_mv(&self, offset_mv: i16) -> Result<()> {
        let clamped = offset_mv.clamp(-150, 0);
        self.verify_msr_unlocked()?;
        Err(OmvixError::UnsupportedHardware(format!(
            "Core undervolt ({clamped} mV, MSR 0x150) requires PawnIO MSR module bridge"
        )))
    }
}
