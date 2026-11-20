use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code};
use omvix_core::{GpuMode, GpuSwitchBackend, OmvixError, Result};

/// Windows HP WMI GPU MUX switch controller (`0x52` / `0x53`).
#[derive(Debug, Default)]
pub struct WindowsGpuSwitchBackend;

impl WindowsGpuSwitchBackend {
    pub fn new() -> Self {
        Self
    }
}

impl GpuSwitchBackend for WindowsGpuSwitchBackend {
    fn get_gpu_mode(&self) -> Result<GpuMode> {
        let bios = HpWmiBios::connect()?;
        let resp = match bios.execute(0x20008, 0x52, &[0u8; 4], OutputSize::Size4) {
            Ok(r) if r.return_code == 0 => r,
            _ => bios.execute(0x20008, 0x21, &[0u8; 4], OutputSize::Size4)?,
        };

        if resp.return_code != 0 || resp.data.is_empty() {
            return Err(OmvixError::WmiCallFailed {
                command: 0x20008,
                command_type: 0x52,
                return_code: resp.return_code,
                message: describe_wmi_return_code(resp.return_code),
            });
        }

        Ok(match resp.data[0] {
            0 => GpuMode::Hybrid,
            1 => GpuMode::Discrete,
            2 => GpuMode::Optimus,
            _ => GpuMode::Unknown,
        })
    }

    fn set_gpu_mode(&self, mode: GpuMode) -> Result<()> {
        let mode_byte = match mode {
            GpuMode::Hybrid => 0x00u8,
            GpuMode::Discrete => 0x01u8,
            GpuMode::Optimus => 0x02u8,
            GpuMode::Unknown => {
                return Err(OmvixError::UnsupportedHardware(
                    "Cannot switch to Unknown GPU mode".to_string(),
                ));
            }
        };

        let bios = HpWmiBios::connect()?;
        let payload = [mode_byte, 0x00, 0x00, 0x00];
        let mut resp = bios.execute(0x20008, 0x53, &payload, OutputSize::Size0)?;
        if resp.return_code == 3 {
            resp = bios.execute(0x20008, 0x52, &payload, OutputSize::Size0)?;
        }

        if resp.return_code == 0 {
            tracing::info!(?mode, "Applied GPU MUX mode (system restart required)");
            Ok(())
        } else {
            Err(OmvixError::WmiCallFailed {
                command: 0x20008,
                command_type: 0x53,
                return_code: resp.return_code,
                message: describe_wmi_return_code(resp.return_code),
            })
        }
    }
}
