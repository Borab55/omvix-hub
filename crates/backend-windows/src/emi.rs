use std::mem::{offset_of, size_of};
use std::slice;
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

/// `{45BD8344-7ED6-49CF-A440-C276C933B053}` (`GUID_DEVICE_ENERGY_METER` from `emi.h`)
const GUID_DEVICE_ENERGY_METER: GUID = GUID::from_values(
    0x45bd8344,
    0x7ed6,
    0x49cf,
    [0xa4, 0x40, 0xc2, 0x76, 0xc9, 0x33, 0xb0, 0x53],
);

const IOCTL_EMI_GET_VERSION: u32 = 0x0022_4000;
const IOCTL_EMI_GET_METADATA_SIZE: u32 = 0x0022_4004;
const IOCTL_EMI_GET_METADATA: u32 = 0x0022_4008;
const IOCTL_EMI_GET_MEASUREMENT: u32 = 0x0022_400C;

const EMI_VERSION_V1: u16 = 1;
const EMI_VERSION_V2: u16 = 2;
const EMI_MEASUREMENT_UNIT_PWH: u32 = 0;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EmiChannelMeasurementData {
    pub absolute_energy: u64,
    pub absolute_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmiChannelInfo {
    pub index: usize,
    pub unit: u32,
    pub name: String,
}

/// Pure parser for Windows `EMI_METADATA_V2` byte buffers (safe bounds-checked slice reads).
pub fn parse_emi_v2_channels(buf: &[u8]) -> Vec<EmiChannelInfo> {
    // EMI_METADATA_V2:
    //   HardwareOEM[16]      (32 bytes)
    //   HardwareModel[16]    (32 bytes)
    //   HardwareRevision     (2 bytes)
    //   ChannelCount         (2 bytes)
    //   Channels[] starts at offset 68
    if buf.len() < 68 {
        return Vec::new();
    }
    let channel_count = u16::from_le_bytes([buf[66], buf[67]]) as usize;
    let mut channels = Vec::with_capacity(channel_count.min(32));
    let mut offset = 68usize;

    for idx in 0..channel_count {
        if offset + 6 > buf.len() {
            break;
        }
        let unit = u32::from_le_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ]);
        let name_size = u16::from_le_bytes([buf[offset + 4], buf[offset + 5]]) as usize;
        let name_start = offset + 6;
        let name_end = name_start.saturating_add(name_size);
        if name_end > buf.len() {
            break;
        }
        let name = decode_utf16le(&buf[name_start..name_end]);
        channels.push(EmiChannelInfo {
            index: idx,
            unit,
            name,
        });
        offset = name_end;
    }

    channels
}

/// Converts deltas of picowatt-hours (`delta_energy`) and 100-ns intervals (`delta_time_100ns`)
/// into average power in Watts.
///
/// `1 pWh = 3.6e-9 J` and `1 tick = 1e-7 s` => `(delta_energy / delta_time) * 0.036`.
pub fn emi_delta_to_watts(delta_energy_pwh: u64, delta_time_100ns: u64) -> Option<f32> {
    if delta_time_100ns == 0 || delta_energy_pwh == 0 {
        return if delta_time_100ns > 0 {
            Some(0.0)
        } else {
            None
        };
    }
    let watts = (delta_energy_pwh as f64 / delta_time_100ns as f64) * 0.036;
    if (0.0..=500.0).contains(&watts) && watts.is_finite() {
        Some(watts as f32)
    } else {
        None
    }
}

fn select_cpu_package_channel(channels: &[EmiChannelInfo]) -> Option<usize> {
    let pwh_channels: Vec<&EmiChannelInfo> = channels
        .iter()
        .filter(|c| c.unit == EMI_MEASUREMENT_UNIT_PWH)
        .collect();

    if pwh_channels.is_empty() {
        return None;
    }

    // Prefer Package / RAPL package domain over individual cores/iGPU/DRAM rails.
    for needle in ["PKG", "PACKAGE", "RAPL_PACKAGE", "CPU_PKG"] {
        if let Some(ch) = pwh_channels
            .iter()
            .find(|c| c.name.to_ascii_uppercase().contains(needle))
        {
            return Some(ch.index);
        }
    }

    for needle in ["IA", "CORE", "CPU"] {
        if let Some(ch) = pwh_channels
            .iter()
            .find(|c| c.name.to_ascii_uppercase().contains(needle))
        {
            return Some(ch.index);
        }
    }

    Some(pwh_channels[0].index)
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let u16s: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&w| w != 0)
        .collect();
    String::from_utf16_lossy(&u16s).trim().to_string()
}

/// Non-intrusive Windows Energy Meter Interface (`EMI`) reader for CPU package power (Watts).
/// Works under VBS / Memory Integrity (HVCI) without kernel drivers or Administrator rights.
pub struct CpuEmiReader {
    handle: HANDLE,
    channel_count: usize,
    target_channel: usize,
    last_sample: Option<EmiChannelMeasurementData>,
    last_watts: Option<f32>,
}

// Win32 file HANDLE is safe to move across threads when guarded by a Mutex.
unsafe impl Send for CpuEmiReader {}

impl Drop for CpuEmiReader {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl CpuEmiReader {
    pub fn open() -> Option<Self> {
        let paths = enumerate_emi_device_paths();
        if paths.is_empty() {
            tracing::debug!("No GUID_DEVICE_ENERGY_METER interfaces exposed by Windows");
            return None;
        }

        let mut fallback: Option<Self> = None;
        for path_w in paths {
            if let Some(reader) = Self::try_open_path(&path_w) {
                if reader.channel_count > 1 {
                    return Some(reader);
                }
                if fallback.is_none() {
                    fallback = Some(reader);
                }
            }
        }
        fallback
    }

    fn try_open_path(path_w: &[u16]) -> Option<Self> {
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path_w.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .ok()?;

        if handle == INVALID_HANDLE_VALUE || handle.is_invalid() {
            return None;
        }

        let version = match ioctl_get_version(handle) {
            Some(v) => v,
            None => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
        };

        let (channel_count, target_channel, channels_summary) = if version == EMI_VERSION_V2 {
            let meta_size = ioctl_get_metadata_size(handle).unwrap_or(0) as usize;
            if !(68..=65536).contains(&meta_size) {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
            let mut meta_buf = vec![0u8; meta_size];
            if !ioctl_get_metadata(handle, &mut meta_buf) {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
            let channels = parse_emi_v2_channels(&meta_buf);
            if channels.is_empty() {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
            let Some(target) = select_cpu_package_channel(&channels) else {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            };
            let summary = channels
                .iter()
                .map(|c| format!("[{}:{}]", c.index, c.name))
                .collect::<Vec<_>>()
                .join(", ");
            (channels.len(), target, summary)
        } else if version == EMI_VERSION_V1 {
            (1usize, 0usize, "[0:V1]".to_string())
        } else {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        };

        let mut reader = Self {
            handle,
            channel_count,
            target_channel,
            last_sample: None,
            last_watts: None,
        };

        // Prime initial measurement so the very next telemetry read can produce Watts cleanly.
        let _ = reader.sample_cpu_power_w();

        tracing::info!(
            version,
            channel_count,
            target_channel,
            channels = %channels_summary,
            "Initialized Windows Energy Meter Interface (EMI) for CPU power"
        );
        Some(reader)
    }

    pub fn sample_cpu_power_w(&mut self) -> Option<f32> {
        let mut buf = vec![EmiChannelMeasurementData::default(); self.channel_count];
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_EMI_GET_MEASUREMENT,
                None,
                0,
                Some(buf.as_mut_ptr().cast()),
                (buf.len() * size_of::<EmiChannelMeasurementData>()) as u32,
                Some(&mut returned),
                None,
            )
        }
        .is_ok();

        if !ok {
            return None;
        }

        let current = *buf.get(self.target_channel)?;
        if let Some(prev) = self.last_sample {
            let dt = current.absolute_time.saturating_sub(prev.absolute_time);
            // Only advance baseline once driver publishes a new counter update (>= 50 ms)
            // so 100 ms UI ticks never suffer divide-by-zero or stale zero deltas.
            if dt >= 500_000 {
                let de = current.absolute_energy.wrapping_sub(prev.absolute_energy);
                self.last_sample = Some(current);
                if let Some(w) = emi_delta_to_watts(de, dt) {
                    self.last_watts = Some(w);
                }
            }
        } else {
            self.last_sample = Some(current);
        }

        self.last_watts
    }
}

fn enumerate_emi_device_paths() -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    let Ok(dev_info) = (unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVICE_ENERGY_METER),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }) else {
        return out;
    };

    for idx in 0..16u32 {
        let mut if_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                dev_info,
                None,
                &GUID_DEVICE_ENERGY_METER,
                idx,
                &mut if_data,
            )
        }
        .is_err()
        {
            break;
        }

        let mut required_size = 0u32;
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &if_data,
                None,
                0,
                Some(&mut required_size),
                None,
            )
        };
        if required_size == 0 || required_size > 16384 {
            continue;
        }

        let mut detail_buf = vec![0u8; required_size as usize];
        let p_detail = detail_buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe {
            (*p_detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &if_data,
                Some(p_detail),
                required_size,
                None,
                None,
            )
        }
        .is_ok()
        {
            let path_offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
            if detail_buf.len() > path_offset {
                let u16_len = (detail_buf.len() - path_offset) / 2;
                let ptr = unsafe { detail_buf.as_ptr().add(path_offset) as *const u16 };
                let slice_w = unsafe { slice::from_raw_parts(ptr, u16_len) };
                let nul_pos = slice_w.iter().position(|&c| c == 0).unwrap_or(u16_len);
                let mut path_w = slice_w[..nul_pos].to_vec();
                path_w.push(0);
                out.push(path_w);
            }
        }
    }

    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(dev_info);
    }
    out
}

fn ioctl_get_version(handle: HANDLE) -> Option<u16> {
    let mut ver = 0u16;
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_EMI_GET_VERSION,
            None,
            0,
            Some((&mut ver as *mut u16).cast()),
            size_of::<u16>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .is_ok();
    ok.then_some(ver)
}

fn ioctl_get_metadata_size(handle: HANDLE) -> Option<u32> {
    let mut sz = 0u32;
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_EMI_GET_METADATA_SIZE,
            None,
            0,
            Some((&mut sz as *mut u32).cast()),
            size_of::<u32>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .is_ok();
    ok.then_some(sz)
}

fn ioctl_get_metadata(handle: HANDLE, out: &mut [u8]) -> bool {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_EMI_GET_METADATA,
            None,
            0,
            Some(out.as_mut_ptr().cast()),
            out.len() as u32,
            Some(&mut returned),
            None,
        )
    }
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_watts_from_emi_pwh_and_100ns_deltas() {
        // 10 W * 1 s = 2,777,777,778 pWh over 10,000,000 ticks
        let w = emi_delta_to_watts(2_777_777_778, 10_000_000).expect("valid watts");
        assert!((w - 10.0).abs() < 0.01, "expected ~10.0W, got {w}");
        assert!(emi_delta_to_watts(100, 0).is_none());
    }

    #[test]
    fn parses_synthetic_emi_v2_metadata_and_picks_pkg() {
        let mut buf = vec![0u8; 68];
        buf[66..68].copy_from_slice(&2u16.to_le_bytes());

        for name in ["IA_CORE0\0", "RAPL_Package0\0"] {
            buf.extend_from_slice(&0u32.to_le_bytes()); // unit = pWh
            let utf16: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            buf.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
            buf.extend_from_slice(&utf16);
        }

        let channels = parse_emi_v2_channels(&buf);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].name, "IA_CORE0");
        assert_eq!(channels[1].name, "RAPL_Package0");
        assert_eq!(select_cpu_package_channel(&channels), Some(1));
    }
}
