use std::fs;
use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

const PAWNIO_DEVICE_PATH: &str = r"\\?\GLOBALROOT\Device\PawnIO";
const IOCTL_PIO_LOAD_BINARY: u32 = (41394u32 << 16) | (0x821u32 << 2);
const IOCTL_PIO_EXECUTE_FN: u32 = (41394u32 << 16) | (0x841u32 << 2);
const FN_NAME_LEN: usize = 32;

const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;

/// Extracts Intel `TjMax` (°C) from `MSR_IA32_TEMPERATURE_TARGET` (`0x1A2`, bits `23:16`).
pub fn decode_intel_tjmax(msr_1a2: u64) -> i32 {
    let raw = ((msr_1a2 >> 16) & 0xFF) as i32;
    if (70..=115).contains(&raw) {
        raw
    } else {
        100
    }
}

/// Decodes Intel package die temperature (°C) from `MSR_IA32_PACKAGE_THERM_STATUS` (`0x1B1`).
pub fn decode_intel_package_temp(msr_1b1: u64, tj_max: i32) -> Option<f32> {
    if ((msr_1b1 >> 31) & 1) == 0 {
        return None;
    }
    let digital_readout = ((msr_1b1 >> 16) & 0x7F) as i32;
    let temp = (tj_max - digital_readout) as f32;
    (10.0..=115.0).contains(&temp).then_some(temp)
}

/// Median-of-3 filter suppressing single-sample MSR self-heating transients.
pub fn median3(a: f32, b: f32, c: f32) -> f32 {
    let mut vals = [a, b, c];
    vals.sort_by(|x, y| x.total_cmp(y));
    vals[1]
}

fn find_intel_msr_module() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local_app_data)
                .join("OmvixHub")
                .join("pawnio")
                .join("IntelMSR.bin"),
        );
    }
    candidates.push(PathBuf::from(
        r"C:\Program Files\PawnIO\modules\IntelMSR.bin",
    ));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("pawnio").join("IntelMSR.bin"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Pure-Rust Win32 `DeviceIoControl` reader for CPU package die temperature via signed PawnIO + `IntelMSR.bin`.
pub struct PawnIoCpuTempReader {
    handle: HANDLE,
    tj_max: i32,
    recent: [f32; 3],
    recent_len: usize,
}

unsafe impl Send for PawnIoCpuTempReader {}

impl Drop for PawnIoCpuTempReader {
    fn drop(&mut self) {
        if !self.handle.is_invalid() && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl PawnIoCpuTempReader {
    /// Opens `\Device\PawnIO` and loads `IntelMSR.bin` if both driver and signed module exist.
    pub fn open() -> Option<Self> {
        let module_path = find_intel_msr_module()?;
        let blob = fs::read(&module_path).ok()?;
        if blob.is_empty() {
            return None;
        }

        let wide_path: Vec<u16> = PAWNIO_DEVICE_PATH
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                0xC000_0000u32, // GENERIC_READ | GENERIC_WRITE
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0x80), // FILE_ATTRIBUTE_NORMAL
                None,
            )
        }
        .ok()?;

        if handle == INVALID_HANDLE_VALUE || handle.is_invalid() {
            return None;
        }

        let mut returned = 0u32;
        let load_ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_PIO_LOAD_BINARY,
                Some(blob.as_ptr().cast()),
                blob.len() as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .is_ok();

        if !load_ok {
            tracing::debug!(
                path = %module_path.display(),
                "PawnIO rejected IntelMSR.bin module load"
            );
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }

        let mut reader = Self {
            handle,
            tj_max: 100,
            recent: [0.0; 3],
            recent_len: 0,
        };

        if let Some(msr_1a2) = reader.read_msr(MSR_IA32_TEMPERATURE_TARGET) {
            reader.tj_max = decode_intel_tjmax(msr_1a2);
        }

        tracing::info!(
            tj_max = reader.tj_max,
            module = %module_path.display(),
            "Initialized PawnIO IntelMSR CPU die temperature reader"
        );
        Some(reader)
    }

    fn read_msr(&self, msr_addr: u64) -> Option<u64> {
        let mut in_buf = [0u8; FN_NAME_LEN + 8];
        let fn_name = b"ioctl_read_msr";
        in_buf[..fn_name.len()].copy_from_slice(fn_name);
        in_buf[FN_NAME_LEN..FN_NAME_LEN + 8].copy_from_slice(&msr_addr.to_le_bytes());

        let mut out_buf = [0u8; 8];
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_PIO_EXECUTE_FN,
                Some(in_buf.as_ptr().cast()),
                in_buf.len() as u32,
                Some(out_buf.as_mut_ptr().cast()),
                out_buf.len() as u32,
                Some(&mut returned),
                None,
            )
        }
        .is_ok();

        if ok && returned >= 8 {
            Some(u64::from_le_bytes(out_buf))
        } else {
            None
        }
    }

    /// Reads smoothed CPU package die temperature (°C) using median-of-3 filtering.
    pub fn read_package_temp_c(&mut self) -> Option<f32> {
        let raw = self.read_msr(MSR_IA32_PACKAGE_THERM_STATUS)?;
        let temp = decode_intel_package_temp(raw, self.tj_max)?;
        if self.recent_len < 3 {
            self.recent[self.recent_len] = temp;
            self.recent_len += 1;
            Some(if self.recent_len == 3 {
                median3(self.recent[0], self.recent[1], self.recent[2])
            } else {
                temp
            })
        } else {
            self.recent[0] = self.recent[1];
            self.recent[1] = self.recent[2];
            self.recent[2] = temp;
            Some(median3(self.recent[0], self.recent[1], self.recent[2]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tjmax_and_package_temp() {
        // TjMax = 100 (0x64) in bits 23:16
        let msr_1a2 = 0x64u64 << 16;
        assert_eq!(decode_intel_tjmax(msr_1a2), 100);
        assert_eq!(decode_intel_tjmax(0), 100);

        // Valid bit 31 = 1, readout = 32 -> 100 - 32 = 68°C
        let msr_1b1 = (1u64 << 31) | (32u64 << 16);
        assert_eq!(decode_intel_package_temp(msr_1b1, 100), Some(68.0));

        // Invalid bit 31 = 0
        assert_eq!(decode_intel_package_temp(32u64 << 16, 100), None);
    }

    #[test]
    fn median3_filters_spikes() {
        assert_eq!(median3(55.0, 99.0, 56.0), 56.0);
        assert_eq!(median3(68.0, 67.0, 69.0), 68.0);
    }
}
