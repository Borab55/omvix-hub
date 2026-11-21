use std::path::PathBuf;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
};
use windows::core::GUID;

fn known_folder_path(rfid: *const GUID) -> Option<PathBuf> {
    let pwstr =
        unsafe { SHGetKnownFolderPath(rfid, KNOWN_FOLDER_FLAG(0), HANDLE::default()) }.ok()?;
    if pwstr.is_null() {
        return None;
    }
    let resolved = unsafe { pwstr.to_string() }.ok();
    unsafe {
        CoTaskMemFree(Some(pwstr.0.cast()));
    }
    resolved.filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Resolves `%ProgramFiles%` cleanly via `SHGetKnownFolderPath(FOLDERID_ProgramFiles)`.
pub fn program_files_dir() -> PathBuf {
    known_folder_path(&FOLDERID_ProgramFiles).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

/// Resolves `%ProgramData%` cleanly via `SHGetKnownFolderPath(FOLDERID_ProgramData)`.
pub fn program_data_dir() -> PathBuf {
    known_folder_path(&FOLDERID_ProgramData).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

/// Admin-protected Omvix Hub installation directory (`C:\Program Files\OmvixHub`).
pub fn install_dir() -> PathBuf {
    program_files_dir().join("OmvixHub")
}

/// Admin-protected GUI binary path (`C:\Program Files\OmvixHub\omvix-hub.exe`) used by elevated startup.
pub fn installed_gui_exe() -> PathBuf {
    install_dir().join("omvix-hub.exe")
}

/// Admin-protected service binary path (`C:\Program Files\OmvixHub\omvix-battery-svc.exe`).
pub fn installed_service_exe() -> PathBuf {
    install_dir().join("omvix-battery-svc.exe")
}

/// Admin-protected PawnIO module directory (`C:\Program Files\OmvixHub\pawnio`).
pub fn pawnio_module_dir() -> PathBuf {
    install_dir().join("pawnio")
}

/// Shared service configuration & log directory (`C:\ProgramData\OmvixHub`).
pub fn omvix_program_data_dir() -> PathBuf {
    program_data_dir().join("OmvixHub")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_folder_paths_resolve_expected_layout() {
        let pf = program_files_dir();
        let pd = program_data_dir();
        assert!(!pf.as_os_str().is_empty());
        assert!(!pd.as_os_str().is_empty());
        assert!(installed_gui_exe().ends_with(PathBuf::from("OmvixHub").join("omvix-hub.exe")));
        assert!(
            installed_service_exe()
                .ends_with(PathBuf::from("OmvixHub").join("omvix-battery-svc.exe"))
        );
        assert!(pawnio_module_dir().ends_with(PathBuf::from("OmvixHub").join("pawnio")));
        assert!(omvix_program_data_dir().ends_with("OmvixHub"));
    }
}
