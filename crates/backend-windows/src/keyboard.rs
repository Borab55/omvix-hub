use crate::system_info::collect_system_identity;
use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code};
use omvix_core::boards::lookup_board;
use omvix_core::{KeyboardBackend, KeyboardType, OmvixError, Result, RgbColor};

/// Windows HP WMI `0x20009` keyboard backlight & RGB controller.
#[derive(Debug, Default)]
pub struct WindowsKeyboardBackend;

impl WindowsKeyboardBackend {
    pub fn new() -> Self {
        Self
    }

    fn scale_color(c: RgbColor, brightness_pct: u8) -> (u8, u8, u8) {
        let factor = brightness_pct.clamp(0, 100) as f32 / 100.0;
        (
            (c.r as f32 * factor).round().clamp(0.0, 255.0) as u8,
            (c.g as f32 * factor).round().clamp(0.0, 255.0) as u8,
            (c.b as f32 * factor).round().clamp(0.0, 255.0) as u8,
        )
    }
}

impl KeyboardBackend for WindowsKeyboardBackend {
    fn detect_keyboard_type(&self) -> Result<KeyboardType> {
        let wmi_type = HpWmiBios::with_cached(|bios| {
            let resp = bios.execute(0x20009, 0x01, &[0u8; 4], OutputSize::Size4)?;
            if resp.return_code == 0 && !resp.data.is_empty() {
                Ok(match resp.data[0] {
                    0 => Some(KeyboardType::WhiteBacklight),
                    1 => Some(KeyboardType::FourZoneRgb),
                    // Observed on Victus 16-r1 (8C99 BIOS F.19): 0x07 0x42 = SingleZoneRgb
                    2 | 7 => Some(KeyboardType::SingleZoneRgb),
                    3 => Some(KeyboardType::PerKeyRgb),
                    other => {
                        tracing::debug!(raw_type = other, "Unmapped WMI keyboard type byte");
                        None
                    }
                })
            } else {
                Ok(None)
            }
        });

        if let Ok(Some(detected)) = wmi_type {
            return Ok(detected);
        }

        if let Ok(id) = collect_system_identity() {
            if let Some(board) = lookup_board(&id.board_id) {
                return Ok(board.expected_keyboard);
            }
        }
        Ok(KeyboardType::Unknown)
    }

    fn set_backlight(
        &self,
        enabled: bool,
        brightness_pct: u8,
        zones: &[RgbColor],
    ) -> Result<()> {
        HpWmiBios::with_cached(|bios| {
            let power_byte = if enabled { 0xE4 } else { 0x64 };
            let toggle = bios.execute(
                0x20009,
                0x05,
                &[power_byte, 0x00, 0x00, 0x00],
                OutputSize::Size0,
            )?;
            if toggle.return_code != 0 {
                return Err(OmvixError::WmiCallFailed {
                    command: 0x20009,
                    command_type: 0x05,
                    return_code: toggle.return_code,
                    message: describe_wmi_return_code(toggle.return_code),
                });
            }

            if !enabled || zones.is_empty() {
                tracing::info!(enabled, "Updated keyboard backlight power state");
                return Ok(());
            }

            let table_resp = bios.execute(0x20009, 0x02, &[0u8; 4], OutputSize::Size128)?;
            if table_resp.return_code != 0 {
                // Single-color white backlights may not expose a color table; toggle already succeeded.
                tracing::debug!(
                    rc = table_resp.return_code,
                    "ColorTable read unsupported (white backlight or locked)"
                );
                return Ok(());
            }

            let mut table = table_resp.data;
            if table.len() < 128 {
                table.resize(128, 0);
            }

            // Standard HP 4-zone RGB layout places R,G,B triplets starting at byte offset 25.
            for zone_idx in 0..4 {
                let color = zones.get(zone_idx).copied().unwrap_or(zones[0]);
                let (r, g, b) = Self::scale_color(color, brightness_pct);
                let offset = 25 + zone_idx * 3;
                table[offset] = r;
                table[offset + 1] = g;
                table[offset + 2] = b;
            }

            let write_resp = bios.execute(0x20009, 0x03, &table, OutputSize::Size0)?;
            if write_resp.return_code != 0 {
                return Err(OmvixError::WmiCallFailed {
                    command: 0x20009,
                    command_type: 0x03,
                    return_code: write_resp.return_code,
                    message: describe_wmi_return_code(write_resp.return_code),
                });
            }

            tracing::info!(brightness_pct, zones = zones.len(), "Applied keyboard RGB colors");
            Ok(())
        })
    }
}
