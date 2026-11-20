use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code, format_hex};
use crate::wmi_com::{WmiConnection, get_u32};
use omvix_core::{BatteryChargeMode, BatteryLimitBackend, BatteryStatus, OmvixError, Result};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

struct BatteryWmiMetrics {
    design_capacity_mwh: Option<u32>,
    full_charge_capacity_mwh: Option<u32>,
    charge_rate_w: Option<f32>,
    voltage_v: Option<f32>,
    remaining_capacity_mwh: Option<u32>,
}

/// Decodes HP WMI `0x2B` (`GBCC`) response bytes into `BatteryChargeMode`.
///
/// Verified against live `8C99` BIOS `F.19` returning `04 FF 01 00` when `Inhibit` is active,
/// as well as standard OMEN/Victus `0x0500` / `0x0300` status words.
pub fn decode_charge_mode(data: &[u8]) -> BatteryChargeMode {
    if data.len() < 2 {
        return BatteryChargeMode::Auto;
    }
    let le = u16::from_le_bytes([data[0], data[1]]);
    match le {
        0x0300 => BatteryChargeMode::Hold,
        0x0500 => BatteryChargeMode::Inhibit,
        _ => match data[0] {
            0x04 | 0x05 => BatteryChargeMode::Inhibit,
            0x02 | 0x03 => BatteryChargeMode::Hold,
            _ => BatteryChargeMode::Auto,
        },
    }
}

/// Windows HP WMI `0x2B` battery charge-limiting and health monitor backend.
#[derive(Debug, Default)]
pub struct WindowsBatteryBackend;

impl WindowsBatteryBackend {
    pub fn new() -> Self {
        Self
    }

    fn read_wmi_metrics() -> BatteryWmiMetrics {
        WmiConnection::with_cached_root_wmi(|wmi| {
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

            let status_row = wmi
                .query(
                    "SELECT ChargeRate, DischargeRate, Voltage, RemainingCapacity FROM BatteryStatus",
                )
                .ok()
                .and_then(|rows| rows.into_iter().next());

            let charge_rate_w = status_row.as_ref().and_then(|o| {
                let c = get_u32(o, "ChargeRate").unwrap_or(0);
                let d = get_u32(o, "DischargeRate").unwrap_or(0);
                if c > 0 && c < 500_000 {
                    Some(c as f32 / 1000.0)
                } else if d > 0 && d < 500_000 {
                    Some(-(d as f32 / 1000.0))
                } else {
                    Some(0.0)
                }
            });

            let voltage_v = status_row
                .as_ref()
                .and_then(|o| get_u32(o, "Voltage"))
                .filter(|&mv| mv > 0 && mv < 50_000)
                .map(|mv| mv as f32 / 1000.0);

            let remaining_capacity_mwh = status_row
                .as_ref()
                .and_then(|o| get_u32(o, "RemainingCapacity"))
                .filter(|&cap| cap > 0 && cap < 1_000_000);

            Ok(BatteryWmiMetrics {
                design_capacity_mwh: design,
                full_charge_capacity_mwh: full,
                charge_rate_w,
                voltage_v,
                remaining_capacity_mwh,
            })
        })
        .unwrap_or(BatteryWmiMetrics {
            design_capacity_mwh: None,
            full_charge_capacity_mwh: None,
            charge_rate_w: None,
            voltage_v: None,
            remaining_capacity_mwh: None,
        })
    }

    fn query_firmware_charge_mode() -> (BatteryChargeMode, Option<String>) {
        let res = HpWmiBios::with_cached(|bios| {
            let resp = bios.execute(0x00001, 0x2B, &[0u8; 4], OutputSize::Size4)?;
            if resp.return_code != 0 || resp.data.len() < 2 {
                return Ok((BatteryChargeMode::Auto, None));
            }
            let mode = decode_charge_mode(&resp.data);
            Ok((mode, Some(format_hex(&resp.data))))
        });
        res.unwrap_or((BatteryChargeMode::Auto, None))
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

        let metrics = Self::read_wmi_metrics();

        let wear_percent = match (
            metrics.design_capacity_mwh,
            metrics.full_charge_capacity_mwh,
        ) {
            (Some(d), Some(f)) if d > 0 => {
                let wear = ((d as f32 - f as f32) / d as f32) * 100.0;
                Some(wear.clamp(0.0, 100.0))
            }
            _ => None,
        };

        let time_remaining_min = if pwr.BatteryLifeTime != u32::MAX && pwr.BatteryLifeTime > 0 {
            Some(pwr.BatteryLifeTime / 60)
        } else if !ac_connected {
            match (metrics.remaining_capacity_mwh, metrics.charge_rate_w) {
                (Some(rem_mwh), Some(rate_w)) if rate_w < -0.5 => {
                    let hrs = (rem_mwh as f32 / 1000.0) / (-rate_w);
                    Some((hrs * 60.0).round().clamp(1.0, 2880.0) as u32)
                }
                _ => None,
            }
        } else {
            None
        };

        let (current_mode, raw_mode_hex) = Self::query_firmware_charge_mode();

        Ok(BatteryStatus {
            present,
            ac_connected,
            charging,
            charge_percent,
            charge_rate_w: metrics.charge_rate_w,
            voltage_v: metrics.voltage_v,
            remaining_capacity_mwh: metrics.remaining_capacity_mwh,
            time_remaining_min,
            design_capacity_mwh: metrics.design_capacity_mwh,
            full_charge_capacity_mwh: metrics.full_charge_capacity_mwh,
            wear_percent,
            current_mode,
            raw_mode_hex,
        })
    }

    fn set_charge_mode(&self, mode: BatteryChargeMode) -> Result<()> {
        let payload: [u8; 4] = match mode {
            BatteryChargeMode::Auto => [0x00, 0x00, 0x00, 0x00],
            BatteryChargeMode::Hold => [0x00, 0x03, 0x00, 0x00],
            BatteryChargeMode::Inhibit => [0x00, 0x05, 0x00, 0x00],
        };

        HpWmiBios::with_cached(|bios| {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_8c99_and_standard_charge_modes() {
        assert_eq!(decode_charge_mode(&[0x04, 0xFF, 0x01, 0x00]), BatteryChargeMode::Inhibit);
        assert_eq!(decode_charge_mode(&[0x00, 0x05, 0x00, 0x00]), BatteryChargeMode::Inhibit);
        assert_eq!(decode_charge_mode(&[0x00, 0x03, 0x00, 0x00]), BatteryChargeMode::Hold);
        assert_eq!(decode_charge_mode(&[0x02, 0xFF, 0x00, 0x00]), BatteryChargeMode::Hold);
        assert_eq!(decode_charge_mode(&[0x00, 0x00, 0x00, 0x00]), BatteryChargeMode::Auto);
    }
}
