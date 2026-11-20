use omvix_core::{OmvixError, Result};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// Directory where Omvix Hub stores verified PawnIO module binaries (`IntelMSR.bin`, etc.).
pub fn pawnio_module_dir() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("OmvixHub").join("pawnio");
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("pawnio");
        }
    }
    PathBuf::from("pawnio")
}

/// Checks whether the PawnIO kernel driver device is reachable or installed on this machine.
pub fn is_pawnio_driver_installed() -> bool {
    let wide_path: Vec<u16> = PAWNIO_DEVICE_PATH
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    if let Ok(handle) = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0xC000_0000u32,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0x80),
            None,
        )
    } {
        if handle != INVALID_HANDLE_VALUE && !handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return true;
        }
    }

    Path::new(r"C:\Program Files\PawnIO\PawnIOUtil.exe").exists()
        || Path::new(r"C:\Program Files\PawnIO").is_dir()
}

/// Returns true when a local `IntelMSR.bin` PawnIO module is already present.
pub fn has_intel_msr_module() -> bool {
    find_intel_msr_module().is_some()
}

/// Downloads and verifies Microsoft-signed PawnIO installer + `IntelMSR.bin` module from GitHub releases.
pub fn install_pawnio_and_module() -> Result<String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let dir = pawnio_module_dir();
    let skip_driver = is_pawnio_driver_installed();

    let script = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$dir = $env:OMVIX_PAWNIO_DIR
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$headers = @{ 'User-Agent' = 'OmvixHub' }
$reboot = $false

if ($env:OMVIX_SKIP_DRIVER -ne '1') {
    $rel = Invoke-RestMethod 'https://api.github.com/repos/namazso/PawnIO.Setup/releases/latest' -Headers $headers -TimeoutSec 25
    $asset = $rel.assets | Where-Object { $_.name -like '*PawnIO_setup*.exe' } | Select-Object -First 1
    if (-not $asset) { throw 'PawnIO_setup.exe asset not found in latest release' }
    $exe = Join-Path $dir 'PawnIO_setup.exe'
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $exe -Headers $headers -TimeoutSec 60
    $sig = Get-AuthenticodeSignature -FilePath $exe
    if ($sig.Status -ne 'Valid') {
        Remove-Item -Force -ErrorAction SilentlyContinue $exe
        throw ('Invalid Authenticode signature on PawnIO_setup.exe: ' + $sig.Status)
    }
    $proc = Start-Process -FilePath $exe -ArgumentList '-install','-silent' -Verb RunAs -Wait -PassThru
    $code = $proc.ExitCode
    Remove-Item -Force -ErrorAction SilentlyContinue $exe
    if ($code -notin @(0, 183, 3010)) {
        throw ('PawnIO_setup.exe failed with exit code ' + $code)
    }
    if ($code -eq 3010) { $reboot = $true }
}

$mrel = Invoke-RestMethod 'https://api.github.com/repos/namazso/PawnIO.Modules/releases/latest' -Headers $headers -TimeoutSec 25
$masset = $mrel.assets | Where-Object { $_.name -like '*.zip' } | Select-Object -First 1
if (-not $masset) { throw 'PawnIO.Modules release zip not found' }
$zip = Join-Path $dir 'modules.zip'
$tmp = Join-Path $dir 'tmp_extract'
Invoke-WebRequest -Uri $masset.browser_download_url -OutFile $zip -Headers $headers -TimeoutSec 60
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
Expand-Archive -Path $zip -DestinationPath $tmp -Force
foreach ($name in @('IntelMSR.bin','AMDFamily17.bin','COPYING')) {
    $found = Get-ChildItem -Path $tmp -Filter $name -Recurse -File | Select-Object -First 1
    if ($found) { Copy-Item -Force $found.FullName (Join-Path $dir $name) }
}
Remove-Item -Force -ErrorAction SilentlyContinue $zip
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
if (-not (Test-Path (Join-Path $dir 'IntelMSR.bin'))) {
    throw 'IntelMSR.bin missing after module extraction'
}
Write-Output ('OK:REBOOT=' + $reboot)
"#;

    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("OMVIX_PAWNIO_DIR", &dir)
        .env("OMVIX_SKIP_DRIVER", if skip_driver { "1" } else { "0" })
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| OmvixError::DriverRequired(format!("Failed to launch PowerShell: {e}")))?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        tracing::warn!(%detail, "PawnIO installation failed");
        return Err(OmvixError::DriverRequired(format!(
            "PawnIO install failed: {detail}"
        )));
    }

    let needs_reboot = stdout.contains("REBOOT=True");
    tracing::info!(dir = %dir.display(), needs_reboot, "PawnIO + IntelMSR.bin installed");
    if needs_reboot {
        Ok("PawnIO installed (reboot recommended if sensor stays inactive)".to_string())
    } else {
        Ok("PawnIO & IntelMSR.bin ready".to_string())
    }
}

fn find_intel_msr_module() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(pawnio_module_dir().join("IntelMSR.bin"));
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
