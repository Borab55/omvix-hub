use crate::system_info::is_process_elevated;
use omvix_core::{OmvixError, Result};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::HSTRING;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "OmvixHub";
const TASK_NAME: &str = "OmvixHub";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// Builds a Task Scheduler 1.2 UTF-16LE XML definition (with BOM) that starts Omvix Hub
/// elevated (`HighestAvailable`, zero UAC prompt) at logon on both AC and battery power.
pub fn build_elevated_task_xml(exe_path: &Path) -> Vec<u8> {
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

    let xml = format!(
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
    <AllowHardTerminate>false</AllowHardTerminate>
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
    );

    let mut bytes = Vec::with_capacity(2 + xml.len() * 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// Returns `true` if the elevated Windows Task Scheduler startup task (`OmvixHub`) exists.
pub fn is_elevated_task_autostart_enabled() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
        if del.0 == 2 { ERROR_SUCCESS } else { del }
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

fn create_elevated_task(exe_path: &Path) -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path =
        std::env::temp_dir().join(format!("omvix_autostart_{}_{}.xml", std::process::id(), nanos));
    let xml_bytes = build_elevated_task_xml(exe_path);
    fs::write(&tmp_path, xml_bytes)?;

    let out = Command::new("schtasks.exe")
        .args(["/Create", "/TN", TASK_NAME, "/XML"])
        .arg(&tmp_path)
        .arg("/F")
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let _ = fs::remove_file(&tmp_path);

    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            Err(OmvixError::PermissionDenied(format!(
                "schtasks /Create failed ({}): {detail}",
                o.status
            )))
        }
        Err(err) => Err(OmvixError::Io(err)),
    }
}

fn delete_elevated_task() {
    let _ = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    if enabled {
        if is_process_elevated() {
            create_elevated_task(exe_path)?;
            // Clean up legacy non-elevated Run entry so Windows does not launch a duplicate copy.
            let _ = set_registry_autostart(false, exe_path);
            tracing::info!("Registered elevated Task Scheduler startup task (OmvixHub)");
            Ok(())
        } else {
            tracing::warn!(
                "Not running as Administrator: falling back to HKCU Run startup (cannot create elevated Scheduled Task)"
            );
            set_registry_autostart(true, exe_path)?;
            tracing::info!("Updated HKCU Run startup entry (non-elevated fallback)");
            Ok(())
        }
    } else {
        delete_elevated_task();
        let reg_res = set_registry_autostart(false, exe_path);
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
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
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
        assert!(xml.contains("<AllowHardTerminate>false</AllowHardTerminate>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains("<Command>C:\\Program Files\\Omvix &amp; Hub\\omvix-hub.exe</Command>"));
        assert!(xml.contains("<WorkingDirectory>C:\\Program Files\\Omvix &amp; Hub</WorkingDirectory>"));
    }
}
