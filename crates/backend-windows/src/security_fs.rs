use omvix_core::{OmvixError, Result};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, BOOL, ERROR_SUCCESS, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
    LookupPrivilegeValueW, ACL, DACL_SECURITY_INFORMATION, LUID_AND_ATTRIBUTES,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SE_PRIVILEGE_ENABLED, SE_RESTORE_NAME, SE_TAKE_OWNERSHIP_NAME,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, DeleteFileW, GetFileAttributesW, GetFileInformationByHandle,
    RemoveDirectoryW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Owner: `BUILTIN\Administrators` (`BA`).
/// Protected DACL (`D:P`): `SYSTEM` full (`FA`), `Administrators` full (`FA`), `Users` read-only (`FR`) with inheritance (`OICI`).
pub const PROGRAM_DATA_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;BU)";

struct LocalFreeSdGuard(PSECURITY_DESCRIPTOR);

impl Drop for LocalFreeSdGuard {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = LocalFree(HLOCAL(self.0 .0));
            }
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Returns `true` if `path` exists and carries `FILE_ATTRIBUTE_REPARSE_POINT` (junction, symlink, etc.).
pub fn is_reparse_point(path: &Path) -> bool {
    let wide = HSTRING::from(path.as_os_str());
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
}

fn enable_privilege(token: HANDLE, name: PCWSTR) {
    let mut luid = Default::default();
    if unsafe { LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid) }.is_err() {
        return;
    }
    let tp = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let _ = unsafe { AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None) };
}

fn enable_ownership_privileges() {
    let mut token = HANDLE::default();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    }
    .is_err()
    {
        return;
    }
    let _guard = HandleGuard(token);
    enable_privilege(token, SE_TAKE_OWNERSHIP_NAME);
    enable_privilege(token, SE_RESTORE_NAME);
}

fn parse_program_data_sd() -> Result<(LocalFreeSdGuard, PSID, *const ACL)> {
    let sddl = HSTRING::from(PROGRAM_DATA_SDDL);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| OmvixError::PermissionDenied(format!("Failed parsing ProgramData SDDL: {e}")))?;

    let guard = LocalFreeSdGuard(sd);
    let mut owner_sid = PSID::default();
    let mut owner_defaulted = BOOL::default();
    unsafe { GetSecurityDescriptorOwner(guard.0, &mut owner_sid, &mut owner_defaulted) }.map_err(
        |e| OmvixError::PermissionDenied(format!("GetSecurityDescriptorOwner failed: {e}")),
    )?;

    let mut dacl_present = BOOL::default();
    let mut dacl_ptr: *mut ACL = std::ptr::null_mut();
    let mut dacl_defaulted = BOOL::default();
    unsafe {
        GetSecurityDescriptorDacl(
            guard.0,
            &mut dacl_present,
            &mut dacl_ptr,
            &mut dacl_defaulted,
        )
    }
    .map_err(|e| OmvixError::PermissionDenied(format!("GetSecurityDescriptorDacl failed: {e}")))?;

    if !dacl_present.as_bool() || dacl_ptr.is_null() {
        return Err(OmvixError::PermissionDenied(
            "Parsed ProgramData SDDL missing DACL".to_string(),
        ));
    }

    Ok((guard, owner_sid, dacl_ptr.cast_const()))
}

fn remove_single_reparse_or_file(path: &Path, attrs: u32) -> Result<()> {
    let wide = HSTRING::from(path.as_os_str());
    let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
    let res = unsafe {
        if is_dir {
            RemoveDirectoryW(PCWSTR(wide.as_ptr()))
        } else {
            DeleteFileW(PCWSTR(wide.as_ptr()))
        }
    };
    res.map_err(|e| {
        OmvixError::PermissionDenied(format!(
            "Failed removing untrusted path {}: {e}",
            path.display()
        ))
    })
}

fn create_dir_with_sd(dir: &Path, sd: PSECURITY_DESCRIPTOR) -> Result<()> {
    if let Some(parent) = dir.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let wide = HSTRING::from(dir.as_os_str());
    unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&sa)) }.map_err(|e| {
        OmvixError::PermissionDenied(format!(
            "CreateDirectoryW({}) with hardened ACL failed: {e}",
            dir.display()
        ))
    })
}

fn apply_owner_and_protected_dacl(path: &Path, owner_sid: PSID, dacl: *const ACL) -> Result<()> {
    let wide = HSTRING::from(path.as_os_str());
    let err = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner_sid,
            PSID::default(),
            Some(dacl),
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(OmvixError::PermissionDenied(format!(
            "SetNamedSecurityInfoW({}) failed with Win32 error {}",
            path.display(),
            err.0
        )));
    }
    Ok(())
}

/// Ensures `C:\ProgramData\OmvixHub` exists, is not a junction/reparse point, is owned by
/// `BUILTIN\Administrators`, and enforces the hardened protected DACL `O:BAD:P(...)`.
///
/// - If `dir` is a reparse point (e.g., attacker-planted junction): removes only the reparse link
///   and recreates the directory atomically with `SECURITY_ATTRIBUTES`.
/// - If `dir` already exists with wrong owner/ACL: repairs owner and resets the protected DACL.
/// - Also inspects/repairs known service files inside `dir`.
pub fn ensure_secure_program_data_dir(dir: &Path) -> Result<()> {
    enable_ownership_privileges();
    let (sd_guard, owner_sid, dacl) = parse_program_data_sd()?;

    let wide = HSTRING::from(dir.as_os_str());
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };

    if attrs == INVALID_FILE_ATTRIBUTES {
        create_dir_with_sd(dir, sd_guard.0)?;
    } else if (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0 {
        tracing::warn!(
            path = %dir.display(),
            "Reparse point detected at ProgramData folder; removing link and recreating securely"
        );
        remove_single_reparse_or_file(dir, attrs)?;
        create_dir_with_sd(dir, sd_guard.0)?;
    } else if (attrs & FILE_ATTRIBUTE_DIRECTORY.0) == 0 {
        tracing::warn!(
            path = %dir.display(),
            "Unexpected file instead of directory at ProgramData path; replacing securely"
        );
        remove_single_reparse_or_file(dir, attrs)?;
        create_dir_with_sd(dir, sd_guard.0)?;
    }
    apply_owner_and_protected_dacl(dir, owner_sid, dacl)?;

    for child_name in [
        "battery-service.toml",
        "battery-service.toml.tmp",
        "battery-service.log",
        "battery-service.log.old",
    ] {
        let child = dir.join(child_name);
        let child_wide = HSTRING::from(child.as_os_str());
        let child_attrs = unsafe { GetFileAttributesW(PCWSTR(child_wide.as_ptr())) };
        if child_attrs == INVALID_FILE_ATTRIBUTES {
            continue;
        }
        if (child_attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
            || (child_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0
        {
            tracing::warn!(
                path = %child.display(),
                "Removing untrusted reparse point or unexpected directory inside ProgramData folder"
            );
            let _ = remove_single_reparse_or_file(&child, child_attrs);
        } else {
            apply_owner_and_protected_dacl(&child, owner_sid, dacl)?;
        }
    }

    Ok(())
}

/// Opens or creates a log file for appending while refusing to follow symlinks or reparse points
/// both on the parent folder and on the target handle itself (`FILE_FLAG_OPEN_REPARSE_POINT`).
pub fn open_log_no_reparse(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && is_reparse_point(parent) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Refusing to write log inside reparse point directory {}",
                    parent.display()
                ),
            ));
        }
    }
    if is_reparse_point(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Refusing to follow reparse point at {}", path.display()),
        ));
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.is_ok()
        && (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Opened handle resolved to reparse point at {}",
                path.display()
            ),
        ));
    }

    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn program_data_sddl_parses_with_admin_owner_and_dacl() {
        let (_guard, owner_sid, dacl) =
            parse_program_data_sd().expect("PROGRAM_DATA_SDDL must parse cleanly");
        assert!(!owner_sid.0.is_null());
        assert!(!dacl.is_null());
    }

    #[test]
    fn open_log_no_reparse_writes_regular_file() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir: PathBuf = std::env::temp_dir().join(format!("omvix_secfs_test_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("test.log");

        assert!(!is_reparse_point(&dir));
        let mut f = open_log_no_reparse(&log_path).expect("regular log opens");
        writeln!(f, "safe log entry").unwrap();
        drop(f);
        assert!(!is_reparse_point(&log_path));
        assert!(fs::read_to_string(&log_path)
            .unwrap()
            .contains("safe log entry"));

        let _ = fs::remove_dir_all(&dir);
    }
}
