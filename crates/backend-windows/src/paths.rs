use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::core::GUID;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{
    SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG,
};

/// Restricts the process default DLL search order to `%SystemRoot%\System32`
/// to mitigate binary/DLL planting attacks. Call at the very top of `main()`.
pub fn harden_dll_search_path() {
    if let Err(err) = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) } {
        eprintln!("Warning: SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) failed: {err}");
    }
}

fn query_win32_dir(f: unsafe fn(Option<&mut [u16]>) -> u32) -> Option<PathBuf> {
    let mut buf = vec![0u16; 260];
    let len = unsafe { f(Some(&mut buf)) } as usize;
    if len == 0 {
        return None;
    }
    if len >= buf.len() {
        buf.resize(len + 1, 0);
        let len2 = unsafe { f(Some(&mut buf)) } as usize;
        if len2 == 0 || len2 >= buf.len() {
            return None;
        }
        return Some(PathBuf::from(OsString::from_wide(&buf[..len2])));
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

/// Resolves `%SystemRoot%\System32` via `GetSystemDirectoryW`.
pub fn system_directory() -> PathBuf {
    query_win32_dir(|buf| unsafe { GetSystemDirectoryW(buf) })
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32"))
}

/// Resolves `%SystemRoot%` via `GetWindowsDirectoryW`.
pub fn windows_directory() -> PathBuf {
    query_win32_dir(|buf| unsafe { GetWindowsDirectoryW(buf) }).unwrap_or_else(|| {
        system_directory()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
    })
}

pub(crate) fn resolve_system_tool(sys32_dir: &Path, win_dir: &Path, name: &str) -> PathBuf {
    if name.eq_ignore_ascii_case("powershell.exe") {
        sys32_dir
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe")
    } else if name.eq_ignore_ascii_case("explorer.exe") {
        win_dir.join("explorer.exe")
    } else {
        sys32_dir.join(name)
    }
}

/// Resolves a trusted Windows system binary or DLL to an absolute path under `%SystemRoot%`,
/// never relying on `PATH` search.
pub fn system_tool(name: &str) -> PathBuf {
    let sys32 = system_directory();
    let win = windows_directory();
    resolve_system_tool(&sys32, &win, name)
}

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
        assert!(installed_service_exe()
            .ends_with(PathBuf::from("OmvixHub").join("omvix-battery-svc.exe")));
        assert!(pawnio_module_dir().ends_with(PathBuf::from("OmvixHub").join("pawnio")));
        assert!(omvix_program_data_dir().ends_with("OmvixHub"));
    }

    #[test]
    fn resolve_system_tool_maps_known_system_binaries() {
        let sys32 = Path::new(r"C:\Windows\System32");
        let win = Path::new(r"C:\Windows");
        assert_eq!(
            resolve_system_tool(sys32, win, "powershell.exe"),
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
        assert_eq!(
            resolve_system_tool(sys32, win, "POWERSHELL.EXE"),
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
        assert_eq!(
            resolve_system_tool(sys32, win, "explorer.exe"),
            PathBuf::from(r"C:\Windows\explorer.exe")
        );
        assert_eq!(
            resolve_system_tool(sys32, win, "schtasks.exe"),
            PathBuf::from(r"C:\Windows\System32\schtasks.exe")
        );
        assert_eq!(
            resolve_system_tool(sys32, win, "shutdown.exe"),
            PathBuf::from(r"C:\Windows\System32\shutdown.exe")
        );
        assert_eq!(
            resolve_system_tool(sys32, win, "nvml.dll"),
            PathBuf::from(r"C:\Windows\System32\nvml.dll")
        );
    }

    #[test]
    fn system_tool_returns_absolute_paths_and_hardening_succeeds() {
        harden_dll_search_path();
        for tool in [
            "powershell.exe",
            "explorer.exe",
            "schtasks.exe",
            "shutdown.exe",
        ] {
            let p = system_tool(tool);
            assert!(p.is_absolute(), "expected absolute path for {tool}: {p:?}");
            assert!(
                p.is_file(),
                "expected system binary to exist on Windows runner: {p:?}"
            );
        }
    }
}
