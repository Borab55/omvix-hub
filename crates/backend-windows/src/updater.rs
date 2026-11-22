use crate::paths;
use crate::security_fs::is_reparse_point;
use crate::system_info::is_process_elevated;
use omvix_core::updater::{
    evaluate_authenticode_signature, parse_github_release_json, parse_sha256_manifest, sha256_hex,
    validate_github_release_url, GITHUB_RELEASES_LATEST_API, MAX_API_JSON_BYTES,
    MAX_CHECKSUM_BYTES, MAX_UPDATE_BINARY_BYTES, PINNED_SIGNER_SUBJECT,
};
use omvix_core::{
    AuthenticodeInfo, OmvixError, ReleaseInfo, Result, SemVer, StagedUpdate, UpdateSignatureStatus,
};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HTTP_404_MARKER: &str = "OMVIX_HTTP_404";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseCheckOutcome {
    UpdateAvailable(Box<ReleaseInfo>),
    UpToDate { latest_tag: String },
    NoReleasesPublished,
}

fn hidden_powershell(script: &str, envs: &[(&str, &str)]) -> Result<Output> {
    let mut cmd = Command::new(crate::paths::system_tool("powershell.exe"));
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW);
    for &(k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().map_err(|e| {
        OmvixError::Io(std::io::Error::new(
            e.kind(),
            format!("PowerShell launch failed: {e}"),
        ))
    })
}

fn schedule_delete_on_reboot(path: &Path) {
    let wide = HSTRING::from(path.as_os_str());
    unsafe {
        let _ = MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}

pub fn check_latest_release(current_version: &str) -> Result<ReleaseCheckOutcome> {
    validate_github_release_url(GITHUB_RELEASES_LATEST_API)?;
    let ua = format!("OmvixHub-Updater/{}", current_version.trim());
    let max_bytes = MAX_API_JSON_BYTES.to_string();

    let script = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor 12288 } catch { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::Tls12 }
$headers = @{
    'User-Agent' = $env:OMVIX_UPDATER_UA
    'Accept' = 'application/vnd.github+json'
}
try {
    $resp = Invoke-WebRequest -Uri $env:OMVIX_UPDATER_API_URL -Headers $headers -TimeoutSec 20 -UseBasicParsing
} catch {
    if ($_.Exception.Response -and $_.Exception.Response.StatusCode.value__ -eq 404) {
        Write-Output 'OMVIX_HTTP_404'
        exit 0
    }
    throw
}
$content = $resp.Content
if ($content -is [byte[]]) {
    $content = [System.Text.Encoding]::UTF8.GetString($content)
}
if ([System.Text.Encoding]::UTF8.GetByteCount([string]$content) -gt [int]$env:OMVIX_MAX_JSON_BYTES) {
    throw 'GitHub release JSON exceeds max size limit'
}
[Console]::Out.Write([string]$content)
"#;

    let out = hidden_powershell(
        script,
        &[
            ("OMVIX_UPDATER_API_URL", GITHUB_RELEASES_LATEST_API),
            ("OMVIX_UPDATER_UA", &ua),
            ("OMVIX_MAX_JSON_BYTES", &max_bytes),
        ],
    )?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(OmvixError::Config(format!(
            "Failed checking GitHub releases: {detail}"
        )));
    }
    if stdout == HTTP_404_MARKER {
        return Ok(ReleaseCheckOutcome::NoReleasesPublished);
    }
    if stdout.len() > MAX_API_JSON_BYTES {
        return Err(OmvixError::Config(format!(
            "GitHub release payload exceeds {MAX_API_JSON_BYTES} bytes"
        )));
    }

    let release = parse_github_release_json(&stdout)?;
    let remote = SemVer::parse(&release.version);
    let local = SemVer::parse(current_version);
    match (remote, local) {
        (Some(r), Some(l)) if r > l => Ok(ReleaseCheckOutcome::UpdateAvailable(Box::new(release))),
        _ => Ok(ReleaseCheckOutcome::UpToDate {
            latest_tag: release.tag_name,
        }),
    }
}

fn resolve_staging_dir() -> Result<PathBuf> {
    let install_dir = paths::install_dir();
    let running_in_install_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.to_path_buf()))
        .is_some_and(|dir| {
            dir.to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(&install_dir.to_string_lossy().to_ascii_lowercase())
        });

    let staging = if is_process_elevated()
        && (paths::installed_gui_exe().exists() || running_in_install_dir)
    {
        if is_reparse_point(&paths::program_files_dir()) || is_reparse_point(&install_dir) {
            return Err(OmvixError::PermissionDenied(
                "Refusing update staging inside reparse point Program Files directory".into(),
            ));
        }
        install_dir.join(".update_staging")
    } else {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("OmvixHub");
        let updates = base.join("updates");
        if is_reparse_point(&base) || is_reparse_point(&updates) {
            return Err(OmvixError::PermissionDenied(
                "Refusing update staging inside reparse point directory".into(),
            ));
        }
        updates.join("staging")
    };

    if is_reparse_point(&staging) {
        return Err(OmvixError::PermissionDenied(format!(
            "Refusing update staging inside reparse point {}",
            staging.display()
        )));
    }
    Ok(staging)
}

pub fn parse_authenticode_output(stdout: &str) -> Result<AuthenticodeInfo> {
    let mut status = None;
    let mut subject = None;
    let mut issuer = None;
    let mut thumbprint = None;
    let mut not_after = None;

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if let Some(val) = line.strip_prefix("STATUS=") {
            let v = val.trim();
            if !v.is_empty() {
                status = Some(v.to_string());
            }
        } else if let Some(val) = line.strip_prefix("SUBJECT=") {
            let v = val.trim();
            if !v.is_empty() {
                subject = Some(v.to_string());
            }
        } else if let Some(val) = line.strip_prefix("ISSUER=") {
            let v = val.trim();
            if !v.is_empty() {
                issuer = Some(v.to_string());
            }
        } else if let Some(val) = line.strip_prefix("THUMBPRINT=") {
            let v = val.trim();
            if !v.is_empty() {
                thumbprint = Some(v.to_string());
            }
        } else if let Some(val) = line.strip_prefix("NOTAFTER=") {
            let v = val.trim();
            if !v.is_empty() {
                not_after = Some(v.to_string());
            }
        }
    }

    let status = status.ok_or_else(|| {
        OmvixError::Config("Authenticode inspection did not return signature status".to_string())
    })?;

    Ok(AuthenticodeInfo {
        status,
        subject,
        issuer,
        thumbprint,
        not_after,
    })
}

fn inspect_authenticode_signature(exe_path: &Path) -> Result<AuthenticodeInfo> {
    if is_reparse_point(exe_path) || !exe_path.is_file() {
        return Err(OmvixError::PermissionDenied(format!(
            "Cannot inspect Authenticode signature on invalid path {}",
            exe_path.display()
        )));
    }
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$sig = Get-AuthenticodeSignature -FilePath $env:OMVIX_SIG_PATH
Write-Output ('STATUS=' + [string]$sig.Status)
$cert = $sig.SignerCertificate
if ($cert) {
    Write-Output ('SUBJECT=' + ([string]$cert.Subject -replace '[\r\n]+', ' '))
    Write-Output ('ISSUER=' + ([string]$cert.Issuer -replace '[\r\n]+', ' '))
    Write-Output ('THUMBPRINT=' + ([string]$cert.Thumbprint -replace '[\r\n]+', ' '))
    Write-Output ('NOTAFTER=' + $cert.NotAfter.ToString('yyyy-MM-dd'))
}
"#;
    let path_str = exe_path.to_string_lossy();
    let out = hidden_powershell(script, &[("OMVIX_SIG_PATH", path_str.as_ref())])?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(OmvixError::PermissionDenied(format!(
            "Get-AuthenticodeSignature failed: {}",
            if !stderr.is_empty() { stderr } else { stdout }
        )));
    }
    parse_authenticode_output(&stdout)
}

pub fn download_and_verify_release(release: &ReleaseInfo) -> Result<StagedUpdate> {
    let checksum_asset = release.checksum_asset.as_ref().ok_or_else(|| {
        OmvixError::Config("Release is missing SHA256SUMS.txt checksum manifest".to_string())
    })?;

    validate_github_release_url(&release.html_url)?;
    validate_github_release_url(&release.binary_asset.download_url)?;
    validate_github_release_url(&checksum_asset.download_url)?;

    let staging = resolve_staging_dir()?;
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    fs::create_dir_all(&staging)?;
    if is_reparse_point(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(OmvixError::PermissionDenied(
            "Staging directory resolved to a reparse point".to_string(),
        ));
    }

    let cleanup_on_err = |dir: &Path| {
        let _ = fs::remove_dir_all(dir);
    };

    let checksum_path = staging.join("SHA256SUMS.txt");
    let raw_asset_name = Path::new(&release.binary_asset.name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "omvix-hub-download.bin".to_string());
    let asset_path = staging.join(format!("download_{raw_asset_name}"));
    let ua = format!("OmvixHub-Updater/{}", env!("CARGO_PKG_VERSION"));
    let max_cs_str = MAX_CHECKSUM_BYTES.to_string();
    let max_bin_str = MAX_UPDATE_BINARY_BYTES.to_string();

    let download_script = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor 12288 } catch { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::Tls12 }
$headers = @{ 'User-Agent' = $env:OMVIX_UA }
Invoke-WebRequest -Uri $env:OMVIX_CHECKSUM_URL -OutFile $env:OMVIX_CHECKSUM_OUT -Headers $headers -TimeoutSec 30 -UseBasicParsing
$csLen = (Get-Item -LiteralPath $env:OMVIX_CHECKSUM_OUT).Length
if ($csLen -le 0 -or $csLen -gt [int64]$env:OMVIX_MAX_CHECKSUM) {
    throw ('Checksum manifest size invalid: ' + $csLen)
}
Invoke-WebRequest -Uri $env:OMVIX_ASSET_URL -OutFile $env:OMVIX_ASSET_OUT -Headers $headers -TimeoutSec 120 -UseBasicParsing
$assetLen = (Get-Item -LiteralPath $env:OMVIX_ASSET_OUT).Length
if ($assetLen -le 0 -or $assetLen -gt [int64]$env:OMVIX_MAX_ASSET) {
    throw ('Release asset size invalid: ' + $assetLen)
}
"#;

    let checksum_out_str = checksum_path.to_string_lossy();
    let asset_out_str = asset_path.to_string_lossy();
    let dl_out = hidden_powershell(
        download_script,
        &[
            ("OMVIX_CHECKSUM_URL", &checksum_asset.download_url),
            ("OMVIX_CHECKSUM_OUT", checksum_out_str.as_ref()),
            ("OMVIX_ASSET_URL", &release.binary_asset.download_url),
            ("OMVIX_ASSET_OUT", asset_out_str.as_ref()),
            ("OMVIX_UA", &ua),
            ("OMVIX_MAX_CHECKSUM", &max_cs_str),
            ("OMVIX_MAX_ASSET", &max_bin_str),
        ],
    );
    match dl_out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let err_txt = String::from_utf8_lossy(&o.stderr).trim().to_string();
            cleanup_on_err(&staging);
            return Err(OmvixError::Config(format!(
                "Update download failed: {err_txt}"
            )));
        }
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e);
        }
    }

    let cs_meta = match fs::metadata(&checksum_path) {
        Ok(m) => m,
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e.into());
        }
    };
    if cs_meta.len() > MAX_CHECKSUM_BYTES as u64 {
        cleanup_on_err(&staging);
        return Err(OmvixError::Config(
            "Downloaded checksum manifest exceeds size limit".into(),
        ));
    }
    let manifest_bytes = match fs::read(&checksum_path) {
        Ok(b) => b,
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e.into());
        }
    };
    let manifest_text = String::from_utf8_lossy(&manifest_bytes).into_owned();
    let Some(expected_asset_sha) =
        parse_sha256_manifest(&manifest_text, &release.binary_asset.name)
    else {
        cleanup_on_err(&staging);
        return Err(OmvixError::Config(format!(
            "SHA256 entry for `{}` not found in checksum manifest",
            release.binary_asset.name
        )));
    };

    let asset_meta = match fs::metadata(&asset_path) {
        Ok(m) => m,
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e.into());
        }
    };
    if asset_meta.len() == 0 || asset_meta.len() > MAX_UPDATE_BINARY_BYTES {
        cleanup_on_err(&staging);
        return Err(OmvixError::Config(
            "Downloaded update asset size invalid".into(),
        ));
    }
    let asset_bytes = match fs::read(&asset_path) {
        Ok(b) => b,
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e.into());
        }
    };
    let actual_asset_sha = sha256_hex(&asset_bytes);
    drop(asset_bytes);

    if !actual_asset_sha.eq_ignore_ascii_case(&expected_asset_sha) {
        cleanup_on_err(&staging);
        return Err(OmvixError::PermissionDenied(format!(
            "SHA-256 verification failed for {}: expected {expected_asset_sha}, got {actual_asset_sha}",
            release.binary_asset.name
        )));
    }

    let staged_exe = staging.join("omvix-hub.exe");
    let (expected_exe_sha, verified_exe_sha) = if release
        .binary_asset
        .name
        .to_ascii_lowercase()
        .ends_with(".zip")
    {
        let extract_script = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($env:OMVIX_ZIP_PATH)
try {
    foreach ($entry in $zip.Entries) {
        if ($entry.FullName -match '(^|[\\/])\.\.([\\/]|$)') {
            throw ('Unsafe path traversal entry in update archive: ' + $entry.FullName)
        }
    }
    $target = $zip.Entries | Where-Object {
        -not [string]::IsNullOrEmpty($_.Name) -and $_.Name -ieq 'omvix-hub.exe'
    } | Select-Object -First 1
    if (-not $target) {
        throw 'omvix-hub.exe missing inside release zip archive'
    }
    if ($target.Length -le 0 -or $target.Length -gt [int64]$env:OMVIX_MAX_ASSET) {
        throw ('Extracted binary size out of bounds: ' + $target.Length)
    }
    $inStream = $target.Open()
    $outStream = [System.IO.File]::Create($env:OMVIX_EXE_OUT)
    try {
        $inStream.CopyTo($outStream)
    } finally {
        $outStream.Dispose()
        $inStream.Dispose()
    }
} finally {
    $zip.Dispose()
}
"#;
        let zip_str = asset_path.to_string_lossy();
        let exe_out_str = staged_exe.to_string_lossy();
        let ex_out = hidden_powershell(
            extract_script,
            &[
                ("OMVIX_ZIP_PATH", zip_str.as_ref()),
                ("OMVIX_EXE_OUT", exe_out_str.as_ref()),
                ("OMVIX_MAX_ASSET", &max_bin_str),
            ],
        );
        let _ = fs::remove_file(&asset_path);
        match ex_out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let err_txt = String::from_utf8_lossy(&o.stderr).trim().to_string();
                cleanup_on_err(&staging);
                return Err(OmvixError::Config(format!(
                    "Failed extracting omvix-hub.exe from archive: {err_txt}"
                )));
            }
            Err(e) => {
                cleanup_on_err(&staging);
                return Err(e);
            }
        }

        let exe_bytes = match fs::read(&staged_exe) {
            Ok(b) => b,
            Err(e) => {
                cleanup_on_err(&staging);
                return Err(e.into());
            }
        };
        let exe_sha = sha256_hex(&exe_bytes);
        if let Some(manifest_exe_sha) = parse_sha256_manifest(&manifest_text, "omvix-hub.exe") {
            if !exe_sha.eq_ignore_ascii_case(&manifest_exe_sha) {
                cleanup_on_err(&staging);
                return Err(OmvixError::PermissionDenied(format!(
                    "Extracted omvix-hub.exe SHA-256 mismatch: expected {manifest_exe_sha}, got {exe_sha}"
                )));
            }
        }
        (exe_sha.clone(), exe_sha)
    } else {
        if let Err(e) = fs::rename(&asset_path, &staged_exe) {
            cleanup_on_err(&staging);
            return Err(e.into());
        }
        (expected_asset_sha, actual_asset_sha)
    };

    let _ = fs::remove_file(&checksum_path);

    if is_reparse_point(&staged_exe) || !staged_exe.is_file() {
        cleanup_on_err(&staging);
        return Err(OmvixError::PermissionDenied(
            "Staged binary missing or resolved to reparse point".into(),
        ));
    }

    let sig_info = match inspect_authenticode_signature(&staged_exe) {
        Ok(info) => info,
        Err(e) => {
            cleanup_on_err(&staging);
            return Err(e);
        }
    };
    let signature = evaluate_authenticode_signature(&sig_info, PINNED_SIGNER_SUBJECT);
    if let UpdateSignatureStatus::Rejected(reason) = &signature {
        let msg = reason.clone();
        cleanup_on_err(&staging);
        return Err(OmvixError::PermissionDenied(msg));
    }

    Ok(StagedUpdate {
        version: release.version.clone(),
        binary_path: staged_exe,
        expected_sha256: expected_exe_sha,
        verified_sha256: verified_exe_sha,
        signature,
        release_url: release.html_url.clone(),
    })
}

fn replace_binary_verified(
    staged_path: &Path,
    target_path: &Path,
    expected_sha256: &str,
) -> Result<PathBuf> {
    let target_parent = target_path.parent().ok_or_else(|| {
        OmvixError::Config(format!(
            "Target path has no parent directory: {}",
            target_path.display()
        ))
    })?;
    if is_reparse_point(staged_path)
        || is_reparse_point(target_path)
        || is_reparse_point(target_parent)
    {
        return Err(OmvixError::PermissionDenied(
            "Refusing binary swap across reparse point paths".into(),
        ));
    }

    let staged_bytes = fs::read(staged_path)?;
    let staged_sha = sha256_hex(&staged_bytes);
    drop(staged_bytes);
    if !staged_sha.eq_ignore_ascii_case(expected_sha256) {
        return Err(OmvixError::PermissionDenied(format!(
            "Staged binary SHA-256 changed before install: expected {expected_sha256}, got {staged_sha}"
        )));
    }

    let mut backup_path = target_parent.join(format!("omvix-hub.exe.old.{}", std::process::id()));
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
        if backup_path.exists() {
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            backup_path = target_parent.join(format!(
                "omvix-hub.exe.old.{}.{}",
                std::process::id(),
                now_ns
            ));
        }
    }

    fs::rename(target_path, &backup_path)?;
    if let Err(err) = fs::copy(staged_path, target_path) {
        let _ = fs::rename(&backup_path, target_path);
        return Err(OmvixError::Io(err));
    }

    let written_bytes = match fs::read(target_path) {
        Ok(b) => b,
        Err(err) => {
            let _ = fs::remove_file(target_path);
            let _ = fs::rename(&backup_path, target_path);
            return Err(OmvixError::Io(err));
        }
    };
    let written_sha = sha256_hex(&written_bytes);
    drop(written_bytes);

    if !written_sha.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(target_path);
        let _ = fs::rename(&backup_path, target_path);
        return Err(OmvixError::PermissionDenied(format!(
            "Post-copy SHA-256 verification failed: expected {expected_sha256}, got {written_sha}"
        )));
    }

    if fs::remove_file(&backup_path).is_err() {
        schedule_delete_on_reboot(&backup_path);
    }

    Ok(target_path.to_path_buf())
}

pub fn apply_staged_update(staged: &StagedUpdate, allow_unsigned_sha256: bool) -> Result<PathBuf> {
    if !staged.can_install(allow_unsigned_sha256) {
        return Err(OmvixError::PermissionDenied(
            "Update installation blocked by Authenticode signature policy".into(),
        ));
    }

    if !staged.binary_path.is_file() || is_reparse_point(&staged.binary_path) {
        return Err(OmvixError::PermissionDenied(
            "Staged update binary is missing or unsafe".into(),
        ));
    }

    let bytes = fs::read(&staged.binary_path)?;
    let current_sha = sha256_hex(&bytes);
    drop(bytes);
    if !current_sha.eq_ignore_ascii_case(&staged.expected_sha256) {
        if let Some(parent) = staged.binary_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        return Err(OmvixError::PermissionDenied(format!(
            "Pre-install SHA-256 mismatch on staged binary: expected {}, got {current_sha}",
            staged.expected_sha256
        )));
    }

    let live_sig = inspect_authenticode_signature(&staged.binary_path)?;
    let live_status = evaluate_authenticode_signature(&live_sig, PINNED_SIGNER_SUBJECT);
    let sig_downgraded =
        matches!(
            staged.signature,
            UpdateSignatureStatus::AuthenticodeValid { .. }
        ) && !matches!(live_status, UpdateSignatureStatus::AuthenticodeValid { .. });
    let sig_allowed = !sig_downgraded
        && match &live_status {
            UpdateSignatureStatus::AuthenticodeValid { .. } => true,
            UpdateSignatureStatus::UnsignedSha256Only => allow_unsigned_sha256,
            UpdateSignatureStatus::Rejected(_) => false,
        };
    if !sig_allowed {
        if let Some(parent) = staged.binary_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        return Err(OmvixError::PermissionDenied(
            "Pre-install signature verification failed on staged binary".into(),
        ));
    }

    let target = std::env::current_exe()?;
    let pf_prefix = paths::program_files_dir()
        .to_string_lossy()
        .to_ascii_lowercase();
    let target_lower = target.to_string_lossy().to_ascii_lowercase();
    if target_lower.starts_with(&pf_prefix) && !is_process_elevated() {
        return Err(OmvixError::PermissionDenied(
            "Administrator privileges are required to update Program Files installation.".into(),
        ));
    }

    replace_binary_verified(&staged.binary_path, &target, &staged.expected_sha256)?;

    if is_process_elevated() {
        let installed = paths::installed_gui_exe();
        if installed.is_file()
            && fs::canonicalize(&target).ok() != fs::canonicalize(&installed).ok()
        {
            if let Err(err) = crate::autostart::install_or_update_gui_copy(&target) {
                tracing::warn!("Failed syncing Program Files copy after update: {err}");
            }
        }
    }

    if let Some(parent) = staged.binary_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }

    Ok(target)
}

fn cleanup_old_files_in_dir(dir: &Path) {
    if !dir.is_dir() || is_reparse_point(dir) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "omvix-hub.exe.old" || name.starts_with("omvix-hub.exe.old.") {
            let p = entry.path();
            if p.is_file() && !is_reparse_point(&p) {
                let _ = fs::remove_file(&p);
            }
        }
    }
}

pub fn cleanup_stale_update_artifacts() {
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            cleanup_old_files_in_dir(parent);
        }
    }
    cleanup_old_files_in_dir(&paths::install_dir());
}

pub fn open_verified_release_url(url: &str) -> Result<()> {
    validate_github_release_url(url)?;
    Command::new(crate::paths::system_tool("explorer.exe"))
        .arg(url)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authenticode_output_variants() {
        let valid = "\
STATUS=Valid
SUBJECT=CN=Omvix Studio
ISSUER=CN=Test Root
THUMBPRINT=ABCD1234
NOTAFTER=2028-01-01
";
        let info = parse_authenticode_output(valid).expect("valid parse");
        assert_eq!(info.status, "Valid");
        assert_eq!(info.subject.as_deref(), Some("CN=Omvix Studio"));
        assert_eq!(info.issuer.as_deref(), Some("CN=Test Root"));
        assert_eq!(info.thumbprint.as_deref(), Some("ABCD1234"));
        assert_eq!(info.not_after.as_deref(), Some("2028-01-01"));

        let unsigned = "STATUS=NotSigned\n";
        let unsigned_info = parse_authenticode_output(unsigned).expect("unsigned parse");
        assert_eq!(unsigned_info.status, "NotSigned");
        assert!(unsigned_info.subject.is_none());

        assert!(parse_authenticode_output("SUBJECT=MissingStatus\n").is_err());
    }

    #[test]
    fn replace_binary_verified_succeeds_and_rolls_back_on_mismatch() {
        let base = std::env::temp_dir().join(format!("omvix_updater_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let target = base.join("omvix-hub.exe");
        let staged = base.join("staged.exe");
        fs::write(&target, b"old-binary-v1").unwrap();
        fs::write(&staged, b"new-binary-v2").unwrap();

        // Wrong expected hash -> fails and preserves original target untouched
        let wrong_sha = "0".repeat(64);
        assert!(replace_binary_verified(&staged, &target, &wrong_sha).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old-binary-v1");

        // Matching hash -> replaces cleanly
        let good_sha = sha256_hex(b"new-binary-v2");
        let swapped = replace_binary_verified(&staged, &target, &good_sha).expect("swap");
        assert_eq!(swapped, target);
        assert_eq!(fs::read(&target).unwrap(), b"new-binary-v2");

        let _ = fs::remove_dir_all(&base);
    }
}
