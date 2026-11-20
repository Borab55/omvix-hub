use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code};
use crate::wmi_com::{WmiConnection, get_u32};
use omvix_core::{BatteryChargeMode, BatteryLimitBackend, BatteryStatus, OmvixError, Result};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Windows HP WMI `0x2B` battery charge-limiting and health monitor backend.
#[derive(Debug, Default)]
pub struct WindowsBatteryBackend;

impl WindowsBatteryBackend {
    pub fn new() -> Self {
        Self
    }

    fn read_wmi_capacities() -> (Option<u32>, Option<u32>, Option<f32>) {
        let Ok(wmi) = WmiConnection::connect(r"root\WMI") else {
            return (None, None, None);
        };

        let design = wmi
            .query("SELECT DesignedCapacity FROM BatteryStaticData")
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|o| get_u32(&o, "DesignedCapacity"));

        let full = wmi
            .query("SELECT FullChargedCapacity FROM BatteryFullChargedCapacity")
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|o| get_u32(&o, "FullChargedCapacity"));

        let rate_w = wmi
            .query("SELECT ChargeRate, DischargeRate FROM BatteryStatus")
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|o| {
                let c = get_u32(&o, "ChargeRate").unwrap_or(0);
                let d = get_u32(&o, "DischargeRate").unwrap_or(0);
                if c > 0 && c < 500_000 {
                    Some(c as f32 / 1000.0)
                } else if d > 0 && d < 500_000 {
                    Some(-(d as f32 / 1000.0))
                } else {
                    Some(0.0)
                }
            });

        (design, full, rate_w)
    }

    fn query_firmware_charge_mode() -> BatteryChargeMode {
        let Ok(bios) = HpWmiBios::connect() else {
            return BatteryChargeMode::Auto;
        };
        let Ok(resp) = bios.execute(0x00001, 0x2B, &[0u8; 4], OutputSize::Size4) else {
            return BatteryChargeMode::Auto;
        };
        if resp.return_code != 0 || resp.data.len() < 2 {
            return BatteryChargeMode::Auto;
        }
        let code = u16::from_le_bytes([resp.data[0], resp.data[1]]);
        match code {
            0x0300 => BatteryChargeMode::Hold,
            0x0500 => BatteryChargeMode::Inhibit,
            _ => BatteryChargeMode::Auto,
        }
    }
}

impl BatteryLimitBackend for WindowsBatteryBackend {
    fn get_battery_status(&self) -> Result<BatteryStatus> {
        let mut pwr = SYSTEM_POWER_STATUS::default();
        unsafe { GetSystemPowerStatus(&mut pwr) }.map_err(|e| {
            OmvixError::WmiUnavailable(format!("GetSystemPowerStatus failed: {e}"))
        })?;

        let present = pwr.BatteryFlag != 255 && (pwr.BatteryFlag & 128) == 0;
        let ac_connected = pwr.ACLineStatus == 1;
        let charging = present && (pwr.BatteryFlag & 8) != 0;
        let charge_percent = if pwr.BatteryLifePercent <= 100 {
            pwr.BatteryLifePercent
        } else {
            0
        };

        let (design_capacity_mwh, full_charge_capacity_mwh, charge_rate_w) =
            Self::read_wmi_capacities();

        let wear_percent = match (design_capacity_mwh, full_charge_capacity_mwh) {
            (Some(d), Some(f)) if d > 0 => {
                let wear = ((d as f32 - f as f32) / d as f32) * 100.0;
                Some(wear.clamp(0.0, 100.0))
            }
            _ => None,
        };

        let current_mode = Self::query_firmware_charge_mode();

        Ok(BatteryStatus {
            present,
            ac_connected,
            charging,
            charge_percent,
            charge_rate_w,
            design_capacity_mwh,
            full_charge_capacity_mwh,
            wear_percent,
            current_mode,
        })
    }

    fn set_charge_mode(&self, mode: BatteryChargeMode) -> Result<()> {
        let payload: [u8; 4] = match mode {
            BatteryChargeMode::Auto => [0x00, 0x00, 0x00, 0x00],
            BatteryChargeMode::Hold => [0x00, 0x03, 0x00, 0x00],
            BatteryChargeMode::Inhibit => [0x00, 0x05, 0x00, 0x00],
        };

        let bios = HpWmiBios::connect()?;
        let resp = bios.execute(0x00002, 0x2B, &payload, OutputSize::Size0)?;
        if resp.return_code == 0 {
            tracing::info!(?mode, "Applied HP WMI 0x2B battery charge mode");
            Ok(())
        } else {
            Err(OmvixError::WmiCallFailed {
                command: 0x00002,
                command_type: 0x2B,
                return_code: resp.return_code,
                message: describe_wmi_return_code(resp.return_code),
            })
        }
    }
}
