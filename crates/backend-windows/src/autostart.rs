use omvix_core::{OmvixError, Result};
use std::path::Path;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::HSTRING;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "OmvixHub";

pub fn is_autostart_enabled() -> bool {
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

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
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
        tracing::info!(enabled, "Updated Windows Run startup entry");
        Ok(())
    } else {
        Err(OmvixError::PermissionDenied(format!(
            "Registry Run update failed ({})",
            res.0
        )))
    }
}
