use omvix_core::config::is_valid_custom_command_syntax;
use omvix_core::{OmvixError, Result};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use windows::core::{Interface, BSTR, HSTRING, PCWSTR, PWSTR, VARIANT};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TOKEN_ADJUST_DEFAULT,
    TOKEN_ADJUST_SESSIONID, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider,
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{
    CreateProcessWithTokenW, OpenProcess, OpenProcessToken, LOGON_WITH_PROFILE,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::{
    IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows, SID_STopLevelBrowser,
    ShellWindows, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetShellWindow, GetWindowThreadProcessId, SW_SHOWNORMAL,
};

struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
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

/// Validates that `target` is a safe `.exe` or `.lnk` file on disk without shell chaining characters.
pub fn validate_launch_target(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim().trim_matches('"').trim();
    if !is_valid_custom_command_syntax(trimmed) {
        return Err(OmvixError::Config(
            "Custom command must point to a valid .exe or .lnk file without shell operators"
                .to_string(),
        ));
    }

    let direct = Path::new(trimmed);
    if direct.is_file() {
        return Ok(direct.to_path_buf());
    }

    if !trimmed.contains('\\') && !trimmed.contains('/') {
        let name_wide = HSTRING::from(trimmed);
        let mut buf = vec![0u16; 1024];
        let len = unsafe {
            SearchPathW(
                PCWSTR::null(),
                PCWSTR(name_wide.as_ptr()),
                PCWSTR::null(),
                Some(&mut buf),
                None,
            )
        } as usize;
        if len > 0 && len < buf.len() {
            let resolved = String::from_utf16_lossy(&buf[..len]);
            let resolved_path = PathBuf::from(&resolved);
            if is_valid_custom_command_syntax(&resolved) && resolved_path.is_file() {
                return Ok(resolved_path);
            }
        }
    }

    Err(OmvixError::Config(format!(
        "Custom command target not found: {trimmed}"
    )))
}

fn validate_args(args: &str) -> Result<String> {
    let trimmed = args.trim();
    if trimmed.contains('\0') || trimmed.contains('\r') || trimmed.contains('\n') {
        return Err(OmvixError::Config(
            "Custom command arguments cannot contain control characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn launch_via_desktop_shell(target: &Path, args: &str, is_lnk: bool) -> Result<()> {
    let target_owned = target.to_string_lossy().to_string();
    let args_owned = args.to_string();
    let dir_owned = if is_lnk {
        None
    } else {
        target.parent().map(|p| p.to_string_lossy().to_string())
    };

    let (tx, rx) = mpsc::channel::<std::result::Result<(), String>>();
    thread::spawn(move || {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let _com = ComGuard(hr.is_ok());
        if hr.is_err() {
            let _ = tx.send(Err(format!("CoInitializeEx failed: 0x{:08X}", hr.0 as u32)));
            return;
        }

        let res: std::result::Result<(), String> = (|| {
            let shell_windows: IShellWindows =
                unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }
                    .map_err(|e| format!("CoCreateInstance(ShellWindows): {e}"))?;

            let loc = VARIANT::default();
            let loc_root = VARIANT::default();
            let mut hwnd_i32 = 0i32;
            let desktop_disp = unsafe {
                shell_windows.FindWindowSW(
                    &loc,
                    &loc_root,
                    SWC_DESKTOP,
                    &mut hwnd_i32,
                    SWFO_NEEDDISPATCH,
                )
            }
            .map_err(|e| format!("FindWindowSW(SWC_DESKTOP): {e}"))?;

            if hwnd_i32 == 0 {
                return Err("Desktop shell window unavailable".to_string());
            }

            let svc_provider: IServiceProvider = desktop_disp
                .cast()
                .map_err(|e| format!("cast IServiceProvider: {e}"))?;
            let shell_browser: IShellBrowser =
                unsafe { svc_provider.QueryService(&SID_STopLevelBrowser) }
                    .map_err(|e| format!("QueryService(SID_STopLevelBrowser): {e}"))?;

            let shell_view = unsafe { shell_browser.QueryActiveShellView() }
                .map_err(|e| format!("QueryActiveShellView: {e}"))?;
            let bg_disp: IDispatch = unsafe { shell_view.GetItemObject(SVGIO_BACKGROUND) }
                .map_err(|e| format!("GetItemObject(SVGIO_BACKGROUND): {e}"))?;
            let folder_view: IShellFolderViewDual = bg_disp
                .cast()
                .map_err(|e| format!("cast IShellFolderViewDual: {e}"))?;
            let app_disp = unsafe { folder_view.Application() }
                .map_err(|e| format!("IShellFolderViewDual::Application: {e}"))?;
            let shell_dispatch: IShellDispatch2 = app_disp
                .cast()
                .map_err(|e| format!("cast IShellDispatch2: {e}"))?;

            let file_bstr = BSTR::from(target_owned.as_str());
            let v_args = if args_owned.is_empty() {
                VARIANT::default()
            } else {
                VARIANT::from(args_owned.as_str())
            };
            let v_dir = match dir_owned.as_deref() {
                Some(d) if !d.is_empty() => VARIANT::from(d),
                _ => VARIANT::default(),
            };
            let v_op = VARIANT::from("open");
            let v_show = VARIANT::from(SW_SHOWNORMAL.0);

            unsafe { shell_dispatch.ShellExecute(&file_bstr, &v_args, &v_dir, &v_op, &v_show) }
                .map_err(|e| format!("IShellDispatch2::ShellExecute: {e}"))?;
            Ok(())
        })();

        let _ = tx.send(res);
    });

    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(msg)) => Err(OmvixError::PermissionDenied(msg)),
        Err(_) => Err(OmvixError::PermissionDenied(
            "Timed out waiting for Desktop Shell COM automation".to_string(),
        )),
    }
}

fn launch_via_explorer_token(target: &Path, args: &str) -> Result<()> {
    let shell_hwnd = unsafe { GetShellWindow() };
    if shell_hwnd == HWND::default() {
        return Err(OmvixError::PermissionDenied(
            "Explorer desktop window not found (cannot acquire unelevated token)".to_string(),
        ));
    }

    let mut shell_pid = 0u32;
    unsafe { GetWindowThreadProcessId(shell_hwnd, Some(&mut shell_pid)) };
    if shell_pid == 0 {
        return Err(OmvixError::PermissionDenied(
            "Could not resolve Explorer PID for unelevated launch".to_string(),
        ));
    }

    let shell_proc_raw =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, shell_pid) }.map_err(
            |e| OmvixError::PermissionDenied(format!("OpenProcess(explorer.exe) failed: {e}")),
        )?;
    let shell_proc = HandleGuard(shell_proc_raw);

    let mut shell_token_raw = HANDLE::default();
    unsafe { OpenProcessToken(shell_proc.0, TOKEN_DUPLICATE, &mut shell_token_raw) }.map_err(
        |e| OmvixError::PermissionDenied(format!("OpenProcessToken(explorer.exe) failed: {e}")),
    )?;
    let shell_token = HandleGuard(shell_token_raw);

    let mut primary_token_raw = HANDLE::default();
    let dup_res = unsafe {
        DuplicateTokenEx(
            shell_token.0,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary_token_raw,
        )
    };
    if dup_res.is_err() {
        unsafe {
            DuplicateTokenEx(
                shell_token.0,
                TOKEN_QUERY
                    | TOKEN_DUPLICATE
                    | TOKEN_ASSIGN_PRIMARY
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_SESSIONID,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary_token_raw,
            )
        }
        .map_err(|e| OmvixError::PermissionDenied(format!("DuplicateTokenEx failed: {e}")))?;
    }
    let primary_token = HandleGuard(primary_token_raw);

    let target_str = target.to_string_lossy();
    let cmdline_str = if args.is_empty() {
        format!("\"{target_str}\"")
    } else {
        format!("\"{target_str}\" {args}")
    };
    let mut cmdline_wide: Vec<u16> = cmdline_str
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let app_wide = HSTRING::from(target.as_os_str());
    let workdir_wide = target.parent().map(|p| HSTRING::from(p.as_os_str()));
    let workdir_pcwstr = workdir_wide
        .as_ref()
        .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr()));

    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessWithTokenW(
            primary_token.0,
            LOGON_WITH_PROFILE,
            PCWSTR(app_wide.as_ptr()),
            PWSTR(cmdline_wide.as_mut_ptr()),
            PROCESS_CREATION_FLAGS(0),
            None,
            workdir_pcwstr,
            &si,
            &mut pi,
        )
    }
    .map_err(|e| OmvixError::PermissionDenied(format!("CreateProcessWithTokenW failed: {e}")))?;

    let _proc_guard = HandleGuard(pi.hProcess);
    let _thread_guard = HandleGuard(pi.hThread);
    Ok(())
}

/// Launches an `.exe` or `.lnk` target WITHOUT elevation (Medium integrity) — never escalates to
/// Administrator even when Omvix Hub itself is running elevated.
///
/// Primary path uses `IShellDispatch2::ShellExecute` via Desktop Shell (supporting `.lnk` natively
/// with its own working directory/arguments). For `.exe` targets, falls back to `explorer.exe`'s
/// non-elevated token + `CreateProcessWithTokenW`. Fails closed if unelevated execution fails.
pub fn launch_unelevated(target_path: &str, args: &str) -> Result<String> {
    let target = validate_launch_target(target_path)?;
    let clean_args = validate_args(args)?;
    let is_lnk = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"));

    match launch_via_desktop_shell(&target, &clean_args, is_lnk) {
        Ok(()) => {
            tracing::info!(
                target = %target.display(),
                "Launched custom command unelevated via IShellDispatch2::ShellExecute"
            );
            Ok(format!("Launched {} (unelevated)", target.display()))
        }
        Err(shell_err) => {
            if is_lnk {
                return Err(OmvixError::PermissionDenied(format!(
                    "Cannot launch shortcut (.lnk) unelevated: {shell_err}"
                )));
            }
            tracing::debug!(%shell_err, "Desktop Shell launch failed; trying explorer token fallback");
            launch_via_explorer_token(&target, &clean_args).map_err(|tok_err| {
                OmvixError::PermissionDenied(format!(
                    "Failed to launch unelevated ({shell_err}; token fallback: {tok_err})"
                ))
            })?;
            tracing::info!(
                target = %target.display(),
                "Launched custom .exe unelevated via explorer.exe token"
            );
            Ok(format!("Launched {} (unelevated)", target.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn validate_launch_target_enforces_safe_extensions_and_existing_files() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_unelev_test_{nanos}"));
        fs::create_dir_all(&dir).unwrap();

        let exe_path = dir.join("helper.exe");
        let lnk_path = dir.join("helper.lnk");
        let bat_path = dir.join("script.bat");
        fs::write(&exe_path, b"").unwrap();
        fs::write(&lnk_path, b"").unwrap();
        fs::write(&bat_path, b"@echo off\n").unwrap();

        assert!(validate_launch_target(&exe_path.to_string_lossy()).is_ok());
        assert!(validate_launch_target(&lnk_path.to_string_lossy()).is_ok());
        assert!(validate_launch_target(&format!("\"{}\"", exe_path.display())).is_ok());

        // Reject shell scripts, chaining operators, and missing files
        assert!(validate_launch_target(&bat_path.to_string_lossy()).is_err());
        assert!(validate_launch_target("cmd /C calc.exe").is_err());
        assert!(validate_launch_target(&format!("{} && calc.exe", exe_path.display())).is_err());
        assert!(validate_launch_target(&dir.join("missing.exe").to_string_lossy()).is_err());

        // Standard Windows system exe via SearchPathW resolves cleanly
        assert!(validate_launch_target("notepad.exe").is_ok());

        let _ = fs::remove_dir_all(&dir);
    }
}
