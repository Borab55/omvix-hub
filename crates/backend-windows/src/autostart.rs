use crate::paths;
use crate::security_fs::{is_reparse_point, same_file, schedule_delete_on_reboot};
use crate::system_info::is_process_elevated;
use omvix_core::{InstalledCopyStatus, OmvixError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use windows::core::{BSTR, HSTRING, VARIANT};
use windows::Win32::Foundation::{ERROR_SUCCESS, RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};
use windows::Win32::System::TaskScheduler::{
    ITaskFolder, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE, TASK_LOGON_GROUP,
};

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "OmvixHub";
const TASK_NAME: &str = "OmvixHub";

struct ComApartmentGuard {
    should_uninit: bool,
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

/// Safe Windows Task Scheduler COM session.
/// Field order guarantees COM interfaces drop before `_com` calls `CoUninitialize`.
struct TaskSchedulerSession {
    root: ITaskFolder,
    _service: ITaskService,
    _com: ComApartmentGuard,
}

impl TaskSchedulerSession {
    fn connect() -> Result<Self> {
        unsafe {
            let init_hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let should_uninit = if init_hr == S_OK || init_hr == S_FALSE {
                true
            } else if init_hr == RPC_E_CHANGED_MODE {
                false
            } else {
                return Err(OmvixError::PermissionDenied(format!(
                    "CoInitializeEx failed: 0x{:08X}",
                    init_hr.0 as u32
                )));
            };
            let com = ComApartmentGuard { should_uninit };

            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(|e| {
                    OmvixError::PermissionDenied(format!("Failed to create ITaskService: {e}"))
                })?;

            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(|e| {
                    OmvixError::PermissionDenied(format!("ITaskService::Connect failed: {e}"))
                })?;

            let root = service.GetFolder(&BSTR::from(r"\")).map_err(|e| {
                OmvixError::PermissionDenied(format!("ITaskService::GetFolder(\\) failed: {e}"))
            })?;

            Ok(Self {
                root,
                _service: service,
                _com: com,
            })
        }
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after = &rest[amp_idx..];
        if let Some(semi_idx) = after.find(';') {
            let entity = &after[..=semi_idx];
            match entity {
                "&amp;" => out.push('&'),
                "&lt;" => out.push('<'),
                "&gt;" => out.push('>'),
                "&quot;" => out.push('"'),
                "&apos;" => out.push('\''),
                _ => out.push_str(entity),
            }
            rest = &after[semi_idx + 1..];
        } else {
            out.push_str(after);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
fn decode_task_xml_bytes(raw: &[u8]) -> String {
    if raw.starts_with(&[0xFF, 0xFE]) && raw.len() >= 2 {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if raw.starts_with(&[0xFE, 0xFF]) && raw.len() >= 2 {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if raw.len() >= 4 && raw.len().is_multiple_of(2) && raw[1] == 0 && raw[3] == 0 {
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let trimmed = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    String::from_utf8_lossy(trimmed).into_owned()
}

/// Parses `<Command>...</Command>` from a Windows Scheduled Task XML string and unescapes entities.
pub fn extract_task_command_from_xml(xml: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let start_tag = "<command>";
    let end_tag = "</command>";
    let start = lower.find(start_tag)? + start_tag.len();
    let end_rel = lower[start..].find(end_tag)?;
    let raw = xml[start..start + end_rel].trim().trim_matches('"').trim();
    let unescaped = xml_unescape(raw);
    (!unescaped.is_empty()).then_some(unescaped)
}

pub fn query_elevated_task_command() -> Option<String> {
    let session = TaskSchedulerSession::connect().ok()?;
    let task = unsafe { session.root.GetTask(&BSTR::from(TASK_NAME)) }.ok()?;
    let xml = unsafe { task.Xml() }.ok()?.to_string();
    extract_task_command_from_xml(&xml)
}

fn paths_equal_ci(a: &Path, b: &Path) -> bool {
    if same_file(a, b) {
        return true;
    }
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim()
            .trim_matches('"')
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Builds the Task Scheduler 1.2 XML string passed to `ITaskFolder::RegisterTask`.
pub fn build_elevated_task_xml_string(exe_path: &Path) -> String {
    let cmd_escaped = xml_escape(&exe_path.to_string_lossy());
    let workdir_xml = exe_path
        .parent()
        .map(|p| {
            format!(
                "\n      <WorkingDirectory>{}</WorkingDirectory>",
                xml_escape(&p.to_string_lossy())
            )
        })
        .unwrap_or_default();

    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Omvix Hub Elevated Startup</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <GroupId>S-1-5-32-545</GroupId>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>{cmd_escaped}</Command>{workdir_xml}
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// Builds a Task Scheduler 1.2 UTF-16LE XML definition (with BOM) that starts Omvix Hub
/// elevated (`HighestAvailable`, zero UAC prompt) at logon on both AC and battery power.
pub fn build_elevated_task_xml(exe_path: &Path) -> Vec<u8> {
    let xml = build_elevated_task_xml_string(exe_path);

    let mut bytes = Vec::with_capacity(2 + xml.len() * 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// Copies or safely self-updates the protected GUI executable under `C:\Program Files\OmvixHub\omvix-hub.exe`.
///
/// If a copy already exists (including when currently running from the installed copy itself),
/// renames the active `.exe` to `omvix-hub.exe.old` first so Windows allows writing the fresh binary,
/// then deletes `.old` or schedules `MOVEFILE_DELAY_UNTIL_REBOOT` cleanup.
pub fn install_or_update_gui_copy(source_exe: &Path) -> Result<PathBuf> {
    if !is_process_elevated() {
        return Err(OmvixError::PermissionDenied(
            "Administrator privileges are required to install or update the Program Files copy."
                .to_string(),
        ));
    }

    let dir = paths::install_dir();
    if is_reparse_point(&dir) {
        return Err(OmvixError::PermissionDenied(format!(
            "Refusing to install into reparse point {}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)?;

    let target = paths::installed_gui_exe();
    let old_base = dir.join("omvix-hub.exe.old");
    if old_base.exists() {
        let _ = fs::remove_file(&old_base);
    }

    if target.exists() {
        if same_file(source_exe, &target) {
            return Ok(target);
        }
        let backup_path = if old_base.exists() {
            dir.join(format!("omvix-hub.exe.old.{}", std::process::id()))
        } else {
            old_base
        };
        fs::rename(&target, &backup_path)?;
        if let Err(err) = fs::copy(source_exe, &target) {
            let _ = fs::remove_file(&target);
            let _ = fs::rename(&backup_path, &target);
            return Err(OmvixError::Io(err));
        }
        if fs::remove_file(&backup_path).is_err() {
            schedule_delete_on_reboot(&backup_path);
        }
    } else {
        fs::copy(source_exe, &target)?;
    }

    tracing::info!(
        source = %source_exe.display(),
        installed = %target.display(),
        "Updated protected Omvix Hub Program Files binary"
    );
    Ok(target)
}

/// Queries whether the protected `Program Files` GUI copy and Scheduled Task exist, match,
/// or are older than the currently running executable.
pub fn query_installed_copy_status() -> InstalledCopyStatus {
    let installed = paths::installed_gui_exe();
    let installed_exists = installed.is_file();
    let running_from_installed = std::env::current_exe()
        .ok()
        .is_some_and(|cur| installed_exists && same_file(&cur, &installed));

    let installed_is_outdated = if !running_from_installed && installed_exists {
        match (
            std::env::current_exe()
                .ok()
                .and_then(|cur| fs::metadata(cur).ok()),
            fs::metadata(&installed).ok(),
        ) {
            (Some(cur_m), Some(inst_m)) => match (cur_m.modified().ok(), inst_m.modified().ok()) {
                (Some(ct), Some(it)) => ct > it || (cur_m.len() != inst_m.len() && ct >= it),
                _ => cur_m.len() != inst_m.len(),
            },
            _ => false,
        }
    } else {
        false
    };

    let task_exists = is_elevated_task_autostart_enabled();
    let task_path_matches = query_elevated_task_command()
        .is_some_and(|cmd| paths_equal_ci(Path::new(&cmd), &installed));

    InstalledCopyStatus {
        installed_path: installed.to_string_lossy().to_string(),
        installed_exists,
        running_from_installed,
        installed_is_outdated,
        task_exists,
        task_path_matches,
    }
}

/// Returns `true` if the elevated Windows Task Scheduler startup task (`OmvixHub`) exists.
pub fn is_elevated_task_autostart_enabled() -> bool {
    TaskSchedulerSession::connect()
        .ok()
        .is_some_and(|session| unsafe { session.root.GetTask(&BSTR::from(TASK_NAME)) }.is_ok())
}

fn is_registry_autostart_enabled() -> bool {
    let subkey = HSTRING::from(RUN_SUBKEY);
    let val_name = HSTRING::from(VALUE_NAME);
    let mut hkey = HKEY::default();
    let open = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, 0, KEY_READ, &mut hkey) };
    if open != ERROR_SUCCESS {
        return false;
    }
    let query = unsafe { RegQueryValueExW(hkey, &val_name, None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    query == ERROR_SUCCESS
}

pub fn is_autostart_enabled() -> bool {
    is_elevated_task_autostart_enabled() || is_registry_autostart_enabled()
}

fn set_registry_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    let subkey = HSTRING::from(RUN_SUBKEY);
    let val_name = HSTRING::from(VALUE_NAME);
    let mut hkey = HKEY::default();
    let open = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, 0, KEY_WRITE, &mut hkey) };
    if open != ERROR_SUCCESS {
        return Err(OmvixError::PermissionDenied(format!(
            "Failed to open HKCU Run key ({})",
            open.0
        )));
    }

    let res = if enabled {
        let quoted = format!("\"{}\"", exe_path.display());
        let wide: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        unsafe { RegSetValueExW(hkey, &val_name, 0, REG_SZ, Some(bytes)) }
    } else {
        let del = unsafe { RegDeleteValueW(hkey, &val_name) };
        // ERROR_FILE_NOT_FOUND (2) is fine when disabling an already absent entry.
        if del.0 == 2 {
            ERROR_SUCCESS
        } else {
            del
        }
    };

    unsafe {
        let _ = RegCloseKey(hkey);
    }

    if res == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(OmvixError::PermissionDenied(format!(
            "Registry Run update failed ({})",
            res.0
        )))
    }
}

pub fn create_elevated_task(exe_path: &Path) -> Result<()> {
    let session = TaskSchedulerSession::connect()?;
    let xml = build_elevated_task_xml_string(exe_path);
    let empty = VARIANT::default();
    unsafe {
        session.root.RegisterTask(
            &BSTR::from(TASK_NAME),
            &BSTR::from(xml.as_str()),
            TASK_CREATE_OR_UPDATE.0,
            &empty,
            &empty,
            TASK_LOGON_GROUP,
            &empty,
        )
    }
    .map_err(|e| OmvixError::PermissionDenied(format!("ITaskFolder::RegisterTask failed: {e}")))?;
    Ok(())
}

fn delete_elevated_task() -> Result<()> {
    let session = TaskSchedulerSession::connect()?;
    match unsafe { session.root.DeleteTask(&BSTR::from(TASK_NAME), 0) } {
        Ok(()) => Ok(()),
        Err(err) if err.code().0 == 0x8007_0002u32 as i32 => Ok(()),
        Err(err) => Err(OmvixError::PermissionDenied(format!(
            "ITaskFolder::DeleteTask failed: {err}"
        ))),
    }
}

/// Verifies on startup that any enabled elevated Scheduled Task points ONLY to the admin-protected
/// `C:\Program Files\OmvixHub\omvix-hub.exe` copy and repairs missing/stale paths (B7 + S1).
pub fn verify_and_repair_autostart_task(current_exe: &Path, config_enabled: bool) -> Result<bool> {
    if !is_process_elevated() {
        return Ok(false);
    }
    let task_exists = is_elevated_task_autostart_enabled();
    if !config_enabled && !task_exists {
        return Ok(false);
    }

    let installed = paths::installed_gui_exe();
    let task_cmd = query_elevated_task_command();
    let cmd_ok = task_cmd
        .as_deref()
        .is_some_and(|c| paths_equal_ci(Path::new(c), &installed));

    if installed.is_file() && cmd_ok {
        return Ok(false);
    }

    if !installed.is_file() {
        install_or_update_gui_copy(current_exe)?;
    }
    create_elevated_task(&installed)?;
    let _ = set_registry_autostart(false, &installed);
    tracing::info!(
        prev_cmd = ?task_cmd,
        repaired_path = %installed.display(),
        "Verified and repaired elevated OmvixHub startup task to protected Program Files binary"
    );
    Ok(true)
}

/// Removes the Windows Scheduled Task, legacy Run key, and protected Program Files GUI copy.
pub fn remove_autostart_and_installed_copy() {
    let _ = delete_elevated_task();
    let installed = paths::installed_gui_exe();
    let _ = set_registry_autostart(false, &installed);

    let dir = paths::install_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let lower = name.to_string_lossy().to_ascii_lowercase();
            if (lower == "omvix-hub.exe" || lower.starts_with("omvix-hub.exe.old"))
                && fs::remove_file(&path).is_err()
            {
                schedule_delete_on_reboot(&path);
            }
        }
    }
    let _ = fs::remove_dir(&dir);
}

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    if enabled {
        if is_process_elevated() {
            let protected_target = install_or_update_gui_copy(exe_path)?;
            create_elevated_task(&protected_target)?;
            // Clean up legacy non-elevated Run entry so Windows does not launch a duplicate copy.
            let _ = set_registry_autostart(false, &protected_target);
            tracing::info!(
                target = %protected_target.display(),
                "Registered elevated Task Scheduler startup task pointing to protected Program Files copy"
            );
            Ok(())
        } else {
            tracing::warn!(
                "Not running as Administrator: falling back to HKCU Run startup (cannot install to Program Files or create elevated Scheduled Task)"
            );
            set_registry_autostart(true, exe_path)?;
            tracing::info!("Updated HKCU Run startup entry (non-elevated fallback)");
            Ok(())
        }
    } else {
        let task_res = delete_elevated_task();
        let reg_res = set_registry_autostart(false, exe_path);
        task_res?;
        tracing::info!("Removed Windows Task Scheduler and HKCU Run startup entries");
        reg_res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_utf16_le_with_bom(bytes: &[u8]) -> String {
        assert!(bytes.len() >= 2);
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16(&units).expect("valid UTF-16LE XML")
    }

    #[test]
    fn xml_escape_handles_special_characters() {
        let escaped = xml_escape(r#"C:\Apps\A&B <Test> '1' "2".exe"#);
        assert_eq!(
            escaped,
            "C:\\Apps\\A&amp;B &lt;Test&gt; &apos;1&apos; &quot;2&quot;.exe"
        );
    }

    #[test]
    fn elevated_task_xml_contains_expected_settings_and_bom() {
        let exe = Path::new(r#"C:\Program Files\Omvix & Hub\omvix-hub.exe"#);
        let bytes = build_elevated_task_xml(exe);
        let xml = decode_utf16_le_with_bom(&bytes);

        assert!(xml.contains("<GroupId>S-1-5-32-545</GroupId>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<AllowHardTerminate>true</AllowHardTerminate>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(
            xml.contains("<Command>C:\\Program Files\\Omvix &amp; Hub\\omvix-hub.exe</Command>")
        );
        assert!(
            xml.contains("<WorkingDirectory>C:\\Program Files\\Omvix &amp; Hub</WorkingDirectory>")
        );
    }

    #[test]
    fn extract_task_command_from_xml_parses_and_unescapes() {
        let exe = Path::new(r#"C:\Program Files\Omvix & Hub\omvix-hub.exe"#);
        let bytes = build_elevated_task_xml(exe);
        let xml = decode_task_xml_bytes(&bytes);
        let parsed = extract_task_command_from_xml(&xml).expect("command tag extracted");
        assert_eq!(parsed, r"C:\Program Files\Omvix & Hub\omvix-hub.exe");
    }

    #[test]
    fn in_memory_task_xml_string_matches_bom_and_connects_com() {
        let exe = Path::new(r#"C:\Program Files\OmvixHub\omvix-hub.exe"#);
        let xml_str = build_elevated_task_xml_string(exe);
        assert!(xml_str.contains("<AllowHardTerminate>true</AllowHardTerminate>"));
        assert_eq!(
            extract_task_command_from_xml(&xml_str).as_deref(),
            Some(r"C:\Program Files\OmvixHub\omvix-hub.exe")
        );
        assert!(
            TaskSchedulerSession::connect().is_ok(),
            "ITaskService COM session should connect to root folder"
        );
    }
}
