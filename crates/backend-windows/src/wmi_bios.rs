use crate::wmi_com::{
    WmiConnection, get_method_in_params, get_object, get_string, get_u8_array, get_u32,
    put_variant, variant_from_u8_array, variant_from_u32,
};
use omvix_core::{OmvixError, Result, WmiProbeEntry};
use std::time::Instant;
use windows::core::{IUnknown, Interface, VARIANT};
use windows::Win32::System::Wmi::IWbemClassObject;

const SIGN_SECU: [u8; 4] = [0x53, 0x45, 0x43, 0x55]; // "SECU"

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSize {
    Size0,
    Size4,
    Size128,
    Size1024,
    Size4096,
}

impl OutputSize {
    pub fn method_name(self) -> &'static str {
        match self {
            Self::Size0 => "hpqBIOSInt0",
            Self::Size4 => "hpqBIOSInt4",
            Self::Size128 => "hpqBIOSInt128",
            Self::Size1024 => "hpqBIOSInt1024",
            Self::Size4096 => "hpqBIOSInt4096",
        }
    }

    pub fn byte_len(self) -> u32 {
        match self {
            Self::Size0 => 0,
            Self::Size4 => 4,
            Self::Size128 => 128,
            Self::Size1024 => 1024,
            Self::Size4096 => 4096,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WmiBiosResponse {
    pub return_code: u32,
    pub data: Vec<u8>,
    pub elapsed_ms: u64,
}

/// Native Windows COM/WMI client for HP's `root\WMI\hpqBIntM` BIOS mailbox.
/// Field order matters: COM class objects MUST drop before `conn` uninitializes COM.
pub struct HpWmiBios {
    hpq_bintm_class: IWbemClassObject,
    hpq_bdata_in_class: IWbemClassObject,
    conn: WmiConnection,
    instance_relpath: String,
}

impl HpWmiBios {
    pub fn connect() -> Result<Self> {
        let conn = WmiConnection::connect(r"root\WMI")?;
        let instances = conn.query("SELECT * FROM hpqBIntM")?;
        let first = instances.into_iter().next().ok_or_else(|| {
            OmvixError::WmiUnavailable("No instances returned for root\\WMI\\hpqBIntM".to_string())
        })?;

        let instance_relpath = get_string(&first, "__RELPATH").unwrap_or_else(|| {
            let inst_name = get_string(&first, "InstanceName").unwrap_or_default();
            let escaped = inst_name.replace('\\', "\\\\").replace('"', "\\\"");
            format!("hpqBIntM.InstanceName=\"{escaped}\"")
        });

        let hpq_bintm_class = conn.get_object("hpqBIntM")?;
        let hpq_bdata_in_class = conn.get_object("hpqBDataIn")?;

        tracing::info!(
            instance_relpath = %instance_relpath,
            "Connected to HP WMI BIOS mailbox (hpqBIntM)"
        );

        Ok(Self {
            hpq_bintm_class,
            hpq_bdata_in_class,
            conn,
            instance_relpath,
        })
    }

    pub fn execute(
        &self,
        command: u32,
        command_type: u32,
        input_data: &[u8],
        out_size: OutputSize,
    ) -> Result<WmiBiosResponse> {
        let start = Instant::now();
        let payload: &[u8] = if input_data.is_empty() {
            &[0u8; 4]
        } else {
            input_data
        };

        let data_in = unsafe {
            self.hpq_bdata_in_class.SpawnInstance(0).map_err(|e| {
                OmvixError::WmiUnavailable(format!("Failed to spawn hpqBDataIn: {e}"))
            })?
        };

        put_variant(&data_in, "Sign", &variant_from_u8_array(&SIGN_SECU)?)?;
        put_variant(&data_in, "Command", &variant_from_u32(command))?;
        put_variant(&data_in, "CommandType", &variant_from_u32(command_type))?;
        put_variant(&data_in, "Size", &variant_from_u32(payload.len() as u32))?;
        put_variant(&data_in, "hpqBData", &variant_from_u8_array(payload)?)?;

        let method_name = out_size.method_name();
        let in_params = get_method_in_params(&self.hpq_bintm_class, method_name)?;
        let data_in_unk: IUnknown = data_in.cast().map_err(|e| {
            OmvixError::WmiUnavailable(format!("Cast hpqBDataIn to IUnknown failed: {e}"))
        })?;
        put_variant(&in_params, "InData", &VARIANT::from(data_in_unk))?;

        let out_params = self
            .conn
            .exec_method(&self.instance_relpath, method_name, Some(&in_params))?
            .ok_or_else(|| {
                OmvixError::WmiUnavailable(format!("{method_name} returned no OutParams"))
            })?;

        let out_data = get_object(&out_params, "OutData").ok_or_else(|| {
            OmvixError::WmiUnavailable(format!("{method_name} OutParams missing OutData"))
        })?;

        let return_code = get_u32(&out_data, "rwReturnCode").ok_or_else(|| {
            OmvixError::WmiUnavailable(format!("{method_name} OutData missing rwReturnCode"))
        })?;
        let data = get_u8_array(&out_data, "Data").unwrap_or_default();
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let preview = format_hex_preview(&data, 32);
        tracing::debug!(
            cmd = %format!("0x{command:X}"),
            cmd_type = %format!("0x{command_type:X}"),
            in_len = payload.len(),
            method = method_name,
            rc = return_code,
            elapsed_ms,
            data_preview = %preview,
            "HP WMI BIOS call completed"
        );

        Ok(WmiBiosResponse {
            return_code,
            data,
            elapsed_ms,
        })
    }

    /// Runs a strictly read-only WMI probe suite across known OMEN/Victus endpoints.
    /// Individual unsupported commands (`rc != 0`) are recorded cleanly without aborting.
    pub fn probe_all_readonly(&self) -> Vec<WmiProbeEntry> {
        let probes: &[(&str, u32, u32, OutputSize)] = &[
            ("SystemDesignData", 0x20008, 0x28, OutputSize::Size128),
            ("FanCount", 0x20008, 0x10, OutputSize::Size4),
            ("FanLevelGet", 0x20008, 0x2D, OutputSize::Size128),
            ("FanTableGet", 0x20008, 0x2F, OutputSize::Size128),
            ("FanMaxGet", 0x20008, 0x26, OutputSize::Size4),
            ("GpuModeGet_New", 0x20008, 0x52, OutputSize::Size4),
            ("GpuModeGet_Legacy", 0x20008, 0x21, OutputSize::Size4),
            ("GpuPowerGet", 0x20008, 0x22, OutputSize::Size4),
            ("ThermalTempGet_23", 0x20008, 0x23, OutputSize::Size4),
            ("ThermalTempGet_01", 0x20008, 0x01, OutputSize::Size4),
            ("KeyboardType", 0x20009, 0x01, OutputSize::Size4),
            ("KeyboardColorTable", 0x20009, 0x02, OutputSize::Size128),
            ("KeyboardBacklightGet", 0x20009, 0x04, OutputSize::Size4),
            ("BatteryChargeControlGet_Std", 0x00001, 0x2B, OutputSize::Size4),
            ("BatteryChargeControlGet_GM", 0x20008, 0x2B, OutputSize::Size4),
        ];

        let zero_in = [0u8; 4];
        probes
            .iter()
            .map(|&(name, cmd, cmd_type, out_size)| {
                let t0 = Instant::now();
                match self.execute(cmd, cmd_type, &zero_in, out_size) {
                    Ok(resp) => WmiProbeEntry {
                        name: name.to_string(),
                        command: cmd,
                        command_type: cmd_type,
                        output_size: out_size.byte_len(),
                        success: resp.return_code == 0,
                        return_code: Some(resp.return_code),
                        data_hex: format_hex(&resp.data),
                        error: if resp.return_code == 0 {
                            None
                        } else {
                            Some(describe_wmi_return_code(resp.return_code))
                        },
                        elapsed_ms: resp.elapsed_ms,
                    },
                    Err(err) => WmiProbeEntry {
                        name: name.to_string(),
                        command: cmd,
                        command_type: cmd_type,
                        output_size: out_size.byte_len(),
                        success: false,
                        return_code: None,
                        data_hex: String::new(),
                        error: Some(err.to_string()),
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    },
                }
            })
            .collect()
    }
}

pub fn describe_wmi_return_code(rc: u32) -> String {
    match rc {
        0 => "OK".to_string(),
        2 => "Wrong signature (0x02)".to_string(),
        3 => "Unknown / unsupported command (0x03)".to_string(),
        4 => "Unsupported command or invalid state (0x04)".to_string(),
        5 => "Invalid parameters / buffer size (0x05)".to_string(),
        other => format!("BIOS return code {other} (0x{other:X})"),
    }
}

pub fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_hex_preview(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        format_hex(bytes)
    } else {
        format!("{} ... ({} bytes)", format_hex(&bytes[..limit]), bytes.len())
    }
}
