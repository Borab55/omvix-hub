use crate::paths;
use crate::security_fs::is_reparse_point;
use crate::system_info::is_process_elevated;
use omvix_core::{OmvixError, Result};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

const PAWNIO_DEVICE_PATH: &str = r"\\?\GLOBALROOT\Device\PawnIO";
const IOCTL_PIO_LOAD_BINARY: u32 = (41394u32 << 16) | (0x821u32 << 2);
const IOCTL_PIO_EXECUTE_FN: u32 = (41394u32 << 16) | (0x841u32 << 2);
const FN_NAME_LEN: usize = 32;

const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;

// Pinned PawnIO supply-chain metadata (verified 2026-11-21)
pub const PAWNIO_SETUP_VERSION: &str = "2.2.0";
pub const PAWNIO_SETUP_URL: &str =
    "https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe";
pub const PAWNIO_SETUP_SHA256: &str =
    "1f519a22e47187f70a1379a48ca604981c4fcf694f4e65b734aaa74a9fba3032";

pub const PAWNIO_SIGNER_SUBJECT: &str =
    "E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU";
pub const PAWNIO_SIGNER_ISSUER: &str =
    "CN=GLOBALTRUST 2015 CODESIGNING 1, OU=GLOBALTRUST Certification Service, O=e-commerce monitoring GmbH, L=Wien, S=Wien, C=AT";
pub const PAWNIO_SIGNER_THUMBPRINT: &str = "F380DCC9F706E2756A5047B832FFE719E1BC35F5";

pub const PAWNIO_MODULES_VERSION: &str = "0.2.12";
pub const PAWNIO_MODULES_URL: &str =
    "https://github.com/namazso/PawnIO.Modules/releases/download/0.2.12/release_0_2_12.zip";
pub const PAWNIO_MODULES_ZIP_SHA256: &str =
    "09698897ce19b92f1043a2e322cfcf0b1fe823e0b576971b178a65244fb0af9d";
pub const INTEL_MSR_BIN_SHA256: &str =
    "30a5a302283de1e080dfee99e686eca91d7e53e911095cb91a320fb961f1ce12";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerCheckResult {
    Match,
    ThumbprintChanged { actual: String },
    Rejected(String),
}

/// Validates Authenticode signer metadata against pinned Subject & Issuer (hard fail) and
/// Thumbprint (warn-only so routine cert renewal after 2027-08-05 does not break verified installs).
pub fn verify_signer_policy(
    status: &str,
    subject: &str,
    issuer: &str,
    thumbprint: &str,
) -> SignerCheckResult {
    if !status.trim().eq_ignore_ascii_case("Valid") {
        return SignerCheckResult::Rejected(format!(
            "Authenticode signature status is `{status}`, expected `Valid`"
        ));
    }
    if subject.trim() != PAWNIO_SIGNER_SUBJECT {
        return SignerCheckResult::Rejected(format!("Signer Subject mismatch: got `{subject}`"));
    }
    if issuer.trim() != PAWNIO_SIGNER_ISSUER {
        return SignerCheckResult::Rejected(format!("Signer Issuer mismatch: got `{issuer}`"));
    }
    let clean_tp = thumbprint.trim();
    if !clean_tp.eq_ignore_ascii_case(PAWNIO_SIGNER_THUMBPRINT) {
        return SignerCheckResult::ThumbprintChanged {
            actual: clean_tp.to_string(),
        };
    }
    SignerCheckResult::Match
}

/// Re-export FIPS 180-4 SHA-256 digest from `omvix_core::updater`.
pub use omvix_core::updater::sha256_hex;

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

    let pf_pawnio = paths::program_files_dir().join("PawnIO");
    pf_pawnio.join("PawnIOUtil.exe").exists()
}

/// Returns true when a verified `IntelMSR.bin` PawnIO module is present.
pub fn has_intel_msr_module() -> bool {
    find_intel_msr_module().is_some()
}

/// Downloads and verifies pinned PawnIO (`2.2.0`) + `PawnIO.Modules` (`0.2.12`) with SHA-256 and
/// Authenticode Subject/Issuer checks into admin-protected `C:\Program Files\OmvixHub\pawnio\`.
pub fn install_pawnio_and_module() -> Result<String> {
    if !is_process_elevated() {
        return Err(OmvixError::PermissionDenied(
            "Administrator privileges are required to install PawnIO into Program Files."
                .to_string(),
        ));
    }

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let install_root = paths::install_dir();
    if is_reparse_point(&install_root) {
        return Err(OmvixError::PermissionDenied(format!(
            "Refusing to write into reparse point {}",
            install_root.display()
        )));
    }
    let dir = paths::pawnio_module_dir();
    if is_reparse_point(&dir) {
        return Err(OmvixError::PermissionDenied(format!(
            "Refusing to write into reparse point {}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)?;

    let skip_driver = is_pawnio_driver_installed();

    let script = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor 12288 } catch { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 }
$dir = $env:OMVIX_PAWNIO_DIR
$staging = Join-Path $dir '.staging'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $staging
New-Item -ItemType Directory -Force -Path $staging | Out-Null
$headers = @{ 'User-Agent' = 'OmvixHub' }
$reboot = $false

try {
    if ($env:OMVIX_SKIP_DRIVER -ne '1') {
        $exe = Join-Path $staging 'PawnIO_setup.exe'
        Invoke-WebRequest -Uri $env:OMVIX_SETUP_URL -OutFile $exe -Headers $headers -TimeoutSec 60 -UseBasicParsing
        $actualSetupSha = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
        if ($actualSetupSha -ne $env:OMVIX_SETUP_SHA256) {
            throw ('PawnIO_setup.exe SHA-256 mismatch: ' + $actualSetupSha)
        }
        $sig = Get-AuthenticodeSignature -FilePath $exe
        if ($sig.Status -ne 'Valid') {
            throw ('Invalid Authenticode signature on PawnIO_setup.exe: ' + $sig.Status)
        }
        $cert = $sig.SignerCertificate
        if (-not $cert) {
            throw 'PawnIO_setup.exe missing SignerCertificate'
        }
        if ($cert.Subject -ne $env:OMVIX_SIGNER_SUBJECT) {
            throw ('PawnIO_setup.exe Subject mismatch: ' + $cert.Subject)
        }
        if ($cert.Issuer -ne $env:OMVIX_SIGNER_ISSUER) {
            throw ('PawnIO_setup.exe Issuer mismatch: ' + $cert.Issuer)
        }
        if ($cert.Thumbprint -ne $env:OMVIX_SIGNER_THUMBPRINT) {
            Write-Output ('WARN_THUMBPRINT=' + $cert.Thumbprint)
        }
        $proc = Start-Process -FilePath $exe -ArgumentList '-install','-silent' -Wait -PassThru
        $code = $proc.ExitCode
        if ($code -notin @(0, 183, 3010)) {
            throw ('PawnIO_setup.exe failed with exit code ' + $code)
        }
        if ($code -eq 3010) { $reboot = $true }
    }

    $zip = Join-Path $staging 'modules.zip'
    $ext = Join-Path $staging 'ext'
    Invoke-WebRequest -Uri $env:OMVIX_MODULES_URL -OutFile $zip -Headers $headers -TimeoutSec 60 -UseBasicParsing
    $actualZipSha = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
    if ($actualZipSha -ne $env:OMVIX_MODULES_ZIP_SHA256) {
        throw ('PawnIO.Modules zip SHA-256 mismatch: ' + $actualZipSha)
    }
    Expand-Archive -Path $zip -DestinationPath $ext -Force
    $msrFound = Get-ChildItem -Path $ext -Filter 'IntelMSR.bin' -Recurse -File | Select-Object -First 1
    if (-not $msrFound) {
        throw 'IntelMSR.bin missing inside modules archive'
    }
    $actualMsrSha = (Get-FileHash -Algorithm SHA256 -Path $msrFound.FullName).Hash.ToLowerInvariant()
    if ($actualMsrSha -ne $env:OMVIX_INTEL_MSR_SHA256) {
        throw ('IntelMSR.bin SHA-256 mismatch: ' + $actualMsrSha)
    }
    foreach ($name in @('IntelMSR.bin','AMDFamily17.bin','COPYING')) {
        $found = Get-ChildItem -Path $ext -Filter $name -Recurse -File | Select-Object -First 1
        if ($found) { Copy-Item -Force $found.FullName (Join-Path $dir $name) }
    }
    Write-Output ('OK:REBOOT=' + $reboot)
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $staging
}
"#;

    let out = Command::new(crate::paths::system_tool("powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("OMVIX_PAWNIO_DIR", &dir)
        .env("OMVIX_SKIP_DRIVER", if skip_driver { "1" } else { "0" })
        .env("OMVIX_SETUP_URL", PAWNIO_SETUP_URL)
        .env("OMVIX_SETUP_SHA256", PAWNIO_SETUP_SHA256)
        .env("OMVIX_SIGNER_SUBJECT", PAWNIO_SIGNER_SUBJECT)
        .env("OMVIX_SIGNER_ISSUER", PAWNIO_SIGNER_ISSUER)
        .env("OMVIX_SIGNER_THUMBPRINT", PAWNIO_SIGNER_THUMBPRINT)
        .env("OMVIX_MODULES_URL", PAWNIO_MODULES_URL)
        .env("OMVIX_MODULES_ZIP_SHA256", PAWNIO_MODULES_ZIP_SHA256)
        .env("OMVIX_INTEL_MSR_SHA256", INTEL_MSR_BIN_SHA256)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| OmvixError::DriverRequired(format!("Failed to launch PowerShell: {e}")))?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    for line in stdout.lines() {
        if let Some(tp) = line.strip_prefix("WARN_THUMBPRINT=") {
            tracing::warn!(
                pinned = PAWNIO_SIGNER_THUMBPRINT,
                actual = tp.trim(),
                "PawnIO signer certificate thumbprint changed (Subject & Issuer matched pinned identity)"
            );
        }
    }

    if !out.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        tracing::warn!(%detail, "Pinned PawnIO installation failed");
        return Err(OmvixError::DriverRequired(format!(
            "PawnIO install failed: {detail}"
        )));
    }

    let msr_path = dir.join("IntelMSR.bin");
    let blob = fs::read(&msr_path)?;
    let digest = sha256_hex(&blob);
    if digest != INTEL_MSR_BIN_SHA256 {
        let _ = fs::remove_file(&msr_path);
        return Err(OmvixError::DriverRequired(format!(
            "Post-install IntelMSR.bin SHA-256 verification failed: {digest}"
        )));
    }

    let needs_reboot = stdout.contains("REBOOT=True");
    tracing::info!(
        dir = %dir.display(),
        setup_ver = PAWNIO_SETUP_VERSION,
        modules_ver = PAWNIO_MODULES_VERSION,
        needs_reboot,
        "Pinned PawnIO + IntelMSR.bin verified & installed"
    );
    if needs_reboot {
        Ok("PawnIO installed (reboot recommended if sensor stays inactive)".to_string())
    } else {
        Ok("PawnIO & IntelMSR.bin verified and ready".to_string())
    }
}

fn read_verified_module(path: &Path) -> Option<Vec<u8>> {
    if !path.is_file() || is_reparse_point(path) {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    if sha256_hex(&bytes) == INTEL_MSR_BIN_SHA256 {
        Some(bytes)
    } else {
        tracing::warn!(
            path = %path.display(),
            "Rejected IntelMSR.bin module failing pinned SHA-256 verification"
        );
        None
    }
}

fn try_migrate_legacy_module(bytes: &[u8], target: &Path) -> bool {
    if !is_process_elevated() {
        return false;
    }
    let Some(parent) = target.parent() else {
        return false;
    };
    if is_reparse_point(&paths::install_dir())
        || is_reparse_point(parent)
        || is_reparse_point(target)
    {
        return false;
    }
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let tmp = target.with_extension("bin.tmp");
    if fs::write(&tmp, bytes).is_err() {
        return false;
    }
    if fs::rename(&tmp, target).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    tracing::info!(
        dest = %target.display(),
        "Migrated verified IntelMSR.bin module into protected Program Files directory"
    );
    true
}

fn find_intel_msr_module() -> Option<PathBuf> {
    let protected = paths::pawnio_module_dir().join("IntelMSR.bin");
    if read_verified_module(&protected).is_some() {
        return Some(protected);
    }

    let system_mod = paths::program_files_dir()
        .join("PawnIO")
        .join("modules")
        .join("IntelMSR.bin");
    if system_mod.is_file() && !is_reparse_point(&system_mod) {
        return Some(system_mod);
    }

    let mut legacy_candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        legacy_candidates.push(
            PathBuf::from(local_app_data)
                .join("OmvixHub")
                .join("pawnio")
                .join("IntelMSR.bin"),
        );
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            legacy_candidates.push(dir.join("pawnio").join("IntelMSR.bin"));
        }
    }

    for legacy in legacy_candidates {
        if let Some(bytes) = read_verified_module(&legacy) {
            if try_migrate_legacy_module(&bytes, &protected) {
                return Some(protected);
            }
            return Some(legacy);
        }
    }

    None
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
    /// Opens `\Device\PawnIO` and loads `IntelMSR.bin` if both driver and verified module exist.
    pub fn open() -> Option<Self> {
        let module_path = find_intel_msr_module()?;
        if is_reparse_point(&module_path) {
            tracing::warn!(
                path = %module_path.display(),
                "Refusing to load IntelMSR.bin from reparse point"
            );
            return None;
        }
        let meta = fs::metadata(&module_path).ok()?;
        if meta.len() == 0 || meta.len() > 4 * 1024 * 1024 {
            tracing::warn!(
                path = %module_path.display(),
                len = meta.len(),
                "Refusing to load IntelMSR.bin with invalid size"
            );
            return None;
        }
        let blob = fs::read(&module_path).ok()?;
        if blob.is_empty() {
            return None;
        }
        let system_mod_dir = paths::program_files_dir().join("PawnIO").join("modules");
        if !module_path.starts_with(&system_mod_dir) && sha256_hex(&blob) != INTEL_MSR_BIN_SHA256 {
            tracing::warn!(
                path = %module_path.display(),
                "Refusing to load IntelMSR.bin failing SHA-256 check"
            );
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

    #[test]
    fn sha256_matches_nist_vectors_and_pinned_hashes_valid() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(PAWNIO_SETUP_SHA256.len(), 64);
        assert_eq!(PAWNIO_MODULES_ZIP_SHA256.len(), 64);
        assert_eq!(INTEL_MSR_BIN_SHA256.len(), 64);
    }

    #[test]
    fn signer_policy_rejects_bad_subject_issuer_and_warns_only_on_thumbprint() {
        assert_eq!(
            verify_signer_policy(
                "Valid",
                PAWNIO_SIGNER_SUBJECT,
                PAWNIO_SIGNER_ISSUER,
                PAWNIO_SIGNER_THUMBPRINT
            ),
            SignerCheckResult::Match
        );

        assert_eq!(
            verify_signer_policy(
                "Valid",
                PAWNIO_SIGNER_SUBJECT,
                PAWNIO_SIGNER_ISSUER,
                "0000111122223333444455556666777788889999"
            ),
            SignerCheckResult::ThumbprintChanged {
                actual: "0000111122223333444455556666777788889999".to_string()
            }
        );

        assert!(matches!(
            verify_signer_policy(
                "NotSigned",
                PAWNIO_SIGNER_SUBJECT,
                PAWNIO_SIGNER_ISSUER,
                PAWNIO_SIGNER_THUMBPRINT
            ),
            SignerCheckResult::Rejected(_)
        ));
        assert!(matches!(
            verify_signer_policy(
                "Valid",
                "CN=Evil",
                PAWNIO_SIGNER_ISSUER,
                PAWNIO_SIGNER_THUMBPRINT
            ),
            SignerCheckResult::Rejected(_)
        ));
        assert!(matches!(
            verify_signer_policy(
                "Valid",
                PAWNIO_SIGNER_SUBJECT,
                "CN=Untrusted CA",
                PAWNIO_SIGNER_THUMBPRINT
            ),
            SignerCheckResult::Rejected(_)
        ));
    }
}
