use crate::battery::WindowsBatteryBackend;
use crate::paths;
use crate::security_fs::{ensure_secure_program_data_dir, is_reparse_point, open_log_no_reparse};
use crate::system_info::is_process_elevated;
use crate::wmi_bios::HpWmiBios;
use chrono::Local;
use omvix_core::battery_care::BatteryCareEngine;
use omvix_core::config::BatteryConfig;
use omvix_core::{
    BatteryChargeMode, BatteryLimitBackend, BatteryServiceInfo, BatteryServiceState, OmvixError,
    Result,
};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
    CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceConfigW,
    QueryServiceStatus, RegisterServiceCtrlHandlerExW, SetServiceStatus,
    StartServiceCtrlDispatcherW, StartServiceW, QUERY_SERVICE_CONFIGW, SC_HANDLE,
    SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE, SERVICE_ACCEPT_POWEREVENT,
    SERVICE_ACCEPT_PRESHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_ALL_ACCESS, SERVICE_AUTO_START,
    SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_POWEREVENT,
    SERVICE_CONTROL_PRESHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_DESCRIPTIONW, SERVICE_ERROR_NORMAL,
    SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
};

pub const SERVICE_NAME: &str = "OmvixBatteryService";
const DISPLAY_NAME: &str = "Omvix Hub Battery Care Service";
const DESCRIPTION: &str =
    "Background HP BIOS WMI (0x2B) battery charge limiter with sleep and shutdown Hold protection.";

const SERVICE_CONTROL_DELETE_ACCESS: u32 = 0x0001_0000;
const OS_ERR_ACCESS_DENIED: i32 = 5;
const OS_ERR_SHARING_VIOLATION: i32 = 32;
const OS_ERR_INSUFFICIENT_BUFFER: i32 = 122;
const OS_ERR_SERVICE_NOT_ACTIVE: i32 = 1062;
const WIN32_ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
const LOG_MAX_BYTES: u64 = 1024 * 1024;

struct ScHandleGuard(SC_HANDLE);

impl Drop for ScHandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }
}

pub fn install_dir() -> PathBuf {
    paths::install_dir()
}

pub fn installed_exe() -> PathBuf {
    paths::installed_service_exe()
}

pub fn data_dir() -> PathBuf {
    paths::omvix_program_data_dir()
}

pub fn config_file_path() -> PathBuf {
    data_dir().join("battery-service.toml")
}

pub fn log_file_path() -> PathBuf {
    data_dir().join("battery-service.log")
}

pub fn service_command_line(exe: &Path) -> String {
    format!("\"{}\" --battery-service", exe.display())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryServiceConfig {
    pub enabled: bool,
    pub mode: BatteryChargeMode,
    pub stop_threshold_pct: u8,
    pub start_threshold_pct: u8,
    pub hold_when_shutdown: bool,
    pub off_min_pct: u8,
    pub one_time_full_charge: bool,
    pub poll_interval_secs: u64,
}

impl Default for BatteryServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: true,
            off_min_pct: 40,
            one_time_full_charge: false,
            poll_interval_secs: 5,
        }
    }
}

impl From<&BatteryConfig> for BatteryServiceConfig {
    fn from(cfg: &BatteryConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            mode: cfg.mode,
            stop_threshold_pct: cfg.stop_threshold_pct,
            start_threshold_pct: cfg.start_threshold_pct,
            hold_when_shutdown: cfg.hold_when_shutdown,
            off_min_pct: 40,
            one_time_full_charge: cfg.one_time_full_charge,
            poll_interval_secs: 5,
        }
    }
}

impl BatteryServiceConfig {
    pub fn to_battery_config(&self) -> BatteryConfig {
        BatteryConfig {
            enabled: self.enabled,
            mode: self.mode,
            stop_threshold_pct: self.stop_threshold_pct,
            start_threshold_pct: self.start_threshold_pct,
            hold_when_shutdown: self.hold_when_shutdown,
            one_time_full_charge: self.one_time_full_charge,
            active_profile_id: None,
            custom_profiles: Vec::new(),
        }
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        toml::from_str(&raw).map_err(|e| OmvixError::Config(e.to_string()))
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let raw = toml::to_string_pretty(self).map_err(|e| OmvixError::Config(e.to_string()))?;
        if path.exists() && fs::read_to_string(path).ok().as_deref() == Some(raw.as_str()) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Decides which HP BIOS `0x2B` charge mode to leave active when entering Sleep, Hibernate, or Shutdown.
pub fn decide_off_state_mode(cfg: &BatteryServiceConfig, charge_pct: u8) -> BatteryChargeMode {
    if !cfg.enabled || !cfg.hold_when_shutdown || cfg.one_time_full_charge {
        return BatteryChargeMode::Auto;
    }
    let floor = cfg.off_min_pct.max(40).max(cfg.start_threshold_pct);
    if charge_pct >= floor {
        BatteryChargeMode::Hold
    } else {
        BatteryChargeMode::Auto
    }
}

fn map_service_state(raw: u32) -> BatteryServiceState {
    if raw == SERVICE_RUNNING.0 {
        BatteryServiceState::Running
    } else if raw == SERVICE_STOPPED.0 {
        BatteryServiceState::Stopped
    } else if raw == SERVICE_START_PENDING.0 {
        BatteryServiceState::Starting
    } else if raw == SERVICE_STOP_PENDING.0 {
        BatteryServiceState::Stopping
    } else {
        BatteryServiceState::Unknown
    }
}

fn query_binary_path(svc: SC_HANDLE) -> Option<String> {
    let mut needed: u32 = 0;
    let first = unsafe { QueryServiceConfigW(svc, None, 0, &mut needed) };
    if first.is_ok() || needed == 0 {
        return None;
    }
    let raw_err = io::Error::last_os_error().raw_os_error();
    if raw_err != Some(OS_ERR_INSUFFICIENT_BUFFER) {
        return None;
    }

    // Allocate 8-byte aligned backing storage for QUERY_SERVICE_CONFIGW + variable strings
    let u64_len = (needed as usize).div_ceil(8);
    let mut aligned_buf = vec![0u64; u64_len];
    let cfg_ptr = aligned_buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
    if unsafe { QueryServiceConfigW(svc, Some(cfg_ptr), needed, &mut needed) }.is_err() {
        return None;
    }

    let bin_pwstr = unsafe { (*cfg_ptr).lpBinaryPathName };
    if bin_pwstr.is_null() {
        return None;
    }
    let s = unsafe { bin_pwstr.to_string() }.ok()?;
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn query_service_info() -> BatteryServiceInfo {
    let exe_path = installed_exe();
    let d_dir = data_dir();
    let l_path = log_file_path();
    let mut info = BatteryServiceInfo {
        service_name: SERVICE_NAME.to_string(),
        state: BatteryServiceState::NotInstalled,
        installed_exe_path: exe_path.to_string_lossy().to_string(),
        active_binary_path: None,
        data_dir_path: d_dir.to_string_lossy().to_string(),
        log_file_path: l_path.to_string_lossy().to_string(),
        exe_exists: exe_path.exists(),
        data_dir_exists: d_dir.exists(),
    };

    let Ok(scm_raw) =
        (unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) })
    else {
        return info;
    };
    let scm = ScHandleGuard(scm_raw);
    let svc_name = HSTRING::from(SERVICE_NAME);
    let svc_res = unsafe {
        OpenServiceW(
            scm.0,
            PCWSTR(svc_name.as_ptr()),
            SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
        )
    };
    let svc_raw = match svc_res {
        Ok(h) => h,
        Err(err) => {
            if err.code() != ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() {
                tracing::debug!("OpenServiceW query returned: {err}");
            }
            return info;
        }
    };
    let svc = ScHandleGuard(svc_raw);

    let mut st = SERVICE_STATUS::default();
    if unsafe { QueryServiceStatus(svc.0, &mut st) }.is_ok() {
        info.state = map_service_state(st.dwCurrentState.0);
    } else {
        info.state = BatteryServiceState::Unknown;
    }
    info.active_binary_path = query_binary_path(svc.0);
    info
}

pub fn is_service_running() -> bool {
    query_service_info().state == BatteryServiceState::Running
}

pub fn is_service_installed() -> bool {
    query_service_info().state != BatteryServiceState::NotInstalled
}

pub fn has_service_leftovers() -> bool {
    installed_exe().exists() || install_dir().exists() || data_dir().exists()
}

pub fn sync_config_if_installed(cfg: &BatteryConfig) -> Result<()> {
    if !is_service_installed() {
        return Ok(());
    }
    ensure_secure_program_data_dir(&data_dir())?;
    BatteryServiceConfig::from(cfg).save_to_path(&config_file_path())
}

fn require_admin(action: &str) -> Result<()> {
    if is_process_elevated() {
        Ok(())
    } else {
        Err(OmvixError::PermissionDenied(format!(
            "Administrator privileges are required to {action}."
        )))
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!(
        (fs::canonicalize(a), fs::canonicalize(b)),
        (Ok(ca), Ok(cb)) if ca == cb
    )
}

fn while_in_use<F>(mut op: F) -> io::Result<()>
where
    F: FnMut() -> io::Result<()>,
{
    let start = Instant::now();
    loop {
        match op() {
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(OS_ERR_ACCESS_DENIED | OS_ERR_SHARING_VIOLATION)
                ) && start.elapsed() < Duration::from_secs(10) =>
            {
                thread::sleep(Duration::from_millis(250));
            }
            result => return result,
        }
    }
}

fn stop_and_wait(svc: SC_HANDLE) -> Result<()> {
    let mut status = SERVICE_STATUS::default();
    if let Err(err) = unsafe { ControlService(svc, SERVICE_CONTROL_STOP, &mut status) } {
        let raw = io::Error::last_os_error().raw_os_error();
        if raw != Some(OS_ERR_SERVICE_NOT_ACTIVE) {
            return Err(OmvixError::WmiUnavailable(format!(
                "Failed to stop {SERVICE_NAME}: {err}"
            )));
        }
        return Ok(());
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut cur = SERVICE_STATUS::default();
        unsafe { QueryServiceStatus(svc, &mut cur) }.map_err(|e| {
            OmvixError::WmiUnavailable(format!("QueryServiceStatus failed during stop: {e}"))
        })?;
        if cur.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(OmvixError::WmiUnavailable(format!(
                "Service {SERVICE_NAME} did not stop within 15 seconds"
            )));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn wait_until_running(svc: SC_HANDLE) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut cur = SERVICE_STATUS::default();
        unsafe { QueryServiceStatus(svc, &mut cur) }.map_err(|e| {
            OmvixError::WmiUnavailable(format!("QueryServiceStatus failed during start: {e}"))
        })?;
        if cur.dwCurrentState == SERVICE_RUNNING {
            return Ok(());
        }
        if cur.dwCurrentState == SERVICE_STOPPED {
            return Err(OmvixError::WmiUnavailable(format!(
                "Service {SERVICE_NAME} stopped immediately on startup"
            )));
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn schedule_delete_on_reboot(path: &Path) -> bool {
    let wide = HSTRING::from(path.as_os_str());
    unsafe {
        MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    }
    .is_ok()
}

pub fn install_or_update_service(cfg: &BatteryConfig) -> Result<String> {
    require_admin("install or update OmvixBatteryService")?;

    let i_dir = install_dir();
    if is_reparse_point(&i_dir) {
        return Err(OmvixError::PermissionDenied(format!(
            "Refusing to install service into reparse point {}",
            i_dir.display()
        )));
    }
    let d_dir = data_dir();
    let target_exe = installed_exe();
    fs::create_dir_all(&i_dir)?;
    ensure_secure_program_data_dir(&d_dir)?;
    BatteryServiceConfig::from(cfg).save_to_path(&config_file_path())?;

    let current_exe = std::env::current_exe()?;

    let scm_raw = unsafe {
        OpenSCManagerW(
            PCWSTR::null(),
            PCWSTR::null(),
            SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE,
        )
    }
    .map_err(|e| OmvixError::PermissionDenied(format!("OpenSCManagerW failed: {e}")))?;
    let scm = ScHandleGuard(scm_raw);

    let svc_name = HSTRING::from(SERVICE_NAME);
    let display_name = HSTRING::from(DISPLAY_NAME);
    let bin_cmd = HSTRING::from(service_command_line(&target_exe));
    let deps_utf16: Vec<u16> = "Winmgmt\0\0".encode_utf16().collect();

    let svc = match unsafe { OpenServiceW(scm.0, PCWSTR(svc_name.as_ptr()), SERVICE_ALL_ACCESS) } {
        Ok(existing) => {
            let guard = ScHandleGuard(existing);
            stop_and_wait(guard.0)?;
            if !same_file(&current_exe, &target_exe) {
                while_in_use(|| fs::copy(&current_exe, &target_exe).map(drop))?;
            }
            unsafe {
                ChangeServiceConfigW(
                    guard.0,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    PCWSTR(bin_cmd.as_ptr()),
                    PCWSTR::null(),
                    None,
                    PCWSTR(deps_utf16.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR(display_name.as_ptr()),
                )
            }
            .map_err(|e| OmvixError::WmiUnavailable(format!("ChangeServiceConfigW failed: {e}")))?;
            guard
        }
        Err(_) => {
            if !same_file(&current_exe, &target_exe) {
                while_in_use(|| fs::copy(&current_exe, &target_exe).map(drop))?;
            }
            let created = unsafe {
                CreateServiceW(
                    scm.0,
                    PCWSTR(svc_name.as_ptr()),
                    PCWSTR(display_name.as_ptr()),
                    SERVICE_ALL_ACCESS,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    PCWSTR(bin_cmd.as_ptr()),
                    PCWSTR::null(),
                    None,
                    PCWSTR(deps_utf16.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                )
            }
            .map_err(|e| OmvixError::WmiUnavailable(format!("CreateServiceW failed: {e}")))?;
            ScHandleGuard(created)
        }
    };

    let mut desc_utf16: Vec<u16> = DESCRIPTION
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let desc = SERVICE_DESCRIPTIONW {
        lpDescription: PWSTR(desc_utf16.as_mut_ptr()),
    };
    let _ = unsafe {
        ChangeServiceConfig2W(
            svc.0,
            SERVICE_CONFIG_DESCRIPTION,
            Some((&desc as *const SERVICE_DESCRIPTIONW).cast()),
        )
    };

    unsafe { StartServiceW(svc.0, None) }
        .map_err(|e| OmvixError::WmiUnavailable(format!("StartServiceW failed: {e}")))?;
    wait_until_running(svc.0)?;

    tracing::info!(exe = %target_exe.display(), "Installed and started OmvixBatteryService");
    Ok(format!("Service active at {}", target_exe.display()))
}

pub fn start_service() -> Result<String> {
    require_admin("start OmvixBatteryService")?;
    let scm_raw = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
        .map_err(|e| OmvixError::PermissionDenied(format!("OpenSCManagerW failed: {e}")))?;
    let scm = ScHandleGuard(scm_raw);
    let svc_name = HSTRING::from(SERVICE_NAME);
    let svc_raw = unsafe {
        OpenServiceW(
            scm.0,
            PCWSTR(svc_name.as_ptr()),
            windows::Win32::System::Services::SERVICE_START | SERVICE_QUERY_STATUS,
        )
    }
    .map_err(|e| OmvixError::WmiUnavailable(format!("OpenServiceW failed: {e}")))?;
    let svc = ScHandleGuard(svc_raw);
    unsafe { StartServiceW(svc.0, None) }
        .map_err(|e| OmvixError::WmiUnavailable(format!("StartServiceW failed: {e}")))?;
    wait_until_running(svc.0)?;
    Ok("OmvixBatteryService started".to_string())
}

pub fn stop_service() -> Result<String> {
    require_admin("stop OmvixBatteryService")?;
    let scm_raw = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
        .map_err(|e| OmvixError::PermissionDenied(format!("OpenSCManagerW failed: {e}")))?;
    let scm = ScHandleGuard(scm_raw);
    let svc_name = HSTRING::from(SERVICE_NAME);
    let svc_raw = unsafe {
        OpenServiceW(
            scm.0,
            PCWSTR(svc_name.as_ptr()),
            windows::Win32::System::Services::SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    }
    .map_err(|e| OmvixError::WmiUnavailable(format!("OpenServiceW failed: {e}")))?;
    let svc = ScHandleGuard(svc_raw);
    stop_and_wait(svc.0)?;
    Ok("OmvixBatteryService stopped (0x2B Auto restored)".to_string())
}

pub fn restart_service() -> Result<String> {
    stop_service()?;
    start_service()?;
    Ok("OmvixBatteryService restarted".to_string())
}

/// Stops and deletes `OmvixBatteryService` from SCM, restores `0x2B Auto`, and removes all files & folders
/// under `C:\Program Files\OmvixHub` and `C:\ProgramData\OmvixHub` with zero leftover traces.
pub fn uninstall_service_zero_trace() -> Result<String> {
    require_admin("uninstall OmvixBatteryService")?;

    if let Ok(scm_raw) =
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
    {
        let scm = ScHandleGuard(scm_raw);
        let svc_name = HSTRING::from(SERVICE_NAME);
        if let Ok(svc_raw) = unsafe {
            OpenServiceW(
                scm.0,
                PCWSTR(svc_name.as_ptr()),
                SERVICE_QUERY_STATUS
                    | windows::Win32::System::Services::SERVICE_STOP
                    | SERVICE_CONTROL_DELETE_ACCESS,
            )
        } {
            let svc = ScHandleGuard(svc_raw);
            let _ = stop_and_wait(svc.0);
            if let Err(err) = unsafe { DeleteService(svc.0) } {
                tracing::warn!("DeleteService warning: {err}");
            }
            drop(svc);

            // Wait briefly for SCM to purge registry service key once handles close
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                let probe =
                    unsafe { OpenServiceW(scm.0, PCWSTR(svc_name.as_ptr()), SERVICE_QUERY_STATUS) };
                match probe {
                    Ok(h) => {
                        let _ = ScHandleGuard(h);
                        thread::sleep(Duration::from_millis(150));
                    }
                    Err(_) => break,
                }
            }
        }
    }

    // Restore factory 0x2B Auto normal charging so battery is never left locked after uninstall
    if let Err(err) = WindowsBatteryBackend::new().set_charge_mode(BatteryChargeMode::Auto) {
        tracing::warn!("Could not reset 0x2B to Auto during uninstall: {err}");
    }

    let mut deferred_reboot = false;
    crate::autostart::remove_autostart_and_installed_copy();
    let pawnio_dir = paths::pawnio_module_dir();
    if pawnio_dir.exists()
        && fs::remove_dir_all(&pawnio_dir).is_err()
        && schedule_delete_on_reboot(&pawnio_dir)
    {
        deferred_reboot = true;
    }

    let exe_path = installed_exe();
    let i_dir = install_dir();
    if exe_path.exists() {
        let current = std::env::current_exe().ok();
        let is_self = current
            .as_deref()
            .is_some_and(|cur| same_file(cur, &exe_path));
        if is_self {
            if schedule_delete_on_reboot(&exe_path) {
                let _ = schedule_delete_on_reboot(&i_dir);
                deferred_reboot = true;
            }
        } else if while_in_use(|| fs::remove_file(&exe_path)).is_err()
            && schedule_delete_on_reboot(&exe_path)
        {
            deferred_reboot = true;
        }
    }
    if i_dir.exists() && fs::remove_dir_all(&i_dir).is_err() {
        let _ = schedule_delete_on_reboot(&i_dir);
    }

    let d_dir = data_dir();
    if d_dir.exists()
        && while_in_use(|| fs::remove_dir_all(&d_dir)).is_err()
        && schedule_delete_on_reboot(&d_dir)
    {
        deferred_reboot = true;
    }

    let msg = if deferred_reboot {
        "Service removed & 0x2B Auto restored; locked files scheduled for deletion on reboot"
            .to_string()
    } else {
        "Service, Program Files & ProgramData completely removed (0x2B Auto restored)".to_string()
    };
    tracing::info!(%msg);
    Ok(msg)
}

// ======================= NATIVE WIN32 SERVICE DISPATCHER & DAEMON LOOP =======================

#[derive(Debug)]
enum DaemonEvent {
    Stop,
    Shutdown(Sender<()>),
    Suspend(Sender<()>),
    Resume,
}

static DAEMON_TX: OnceLock<Mutex<Option<Sender<DaemonEvent>>>> = OnceLock::new();
static SVC_STATUS_HANDLE: OnceLock<usize> = OnceLock::new();

fn set_scm_state(state: u32, controls: u32, exit_code: u32, wait_hint_ms: u32) {
    let Some(&raw_h) = SVC_STATUS_HANDLE.get() else {
        return;
    };
    let handle = SERVICE_STATUS_HANDLE(raw_h as *mut c_void);
    if handle.is_invalid() {
        return;
    }
    let st = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE(state),
        dwControlsAccepted: controls,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint_ms,
    };
    unsafe {
        let _ = SetServiceStatus(handle, &st);
    }
}

fn daemon_log(line: &str) {
    let path = log_file_path();
    let Some(parent) = path.parent() else {
        return;
    };
    if is_reparse_point(parent) {
        return;
    }
    if !parent.exists() {
        let _ = ensure_secure_program_data_dir(parent);
    }
    if !is_reparse_point(&path) && fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let backup = path.with_extension("log.old");
        if !is_reparse_point(&backup) {
            let _ = fs::rename(&path, backup);
        }
    }
    if let Ok(mut file) = open_log_no_reparse(&path) {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "{ts} | {line}");
    }
}

fn send_daemon_event(ev: DaemonEvent) {
    if let Some(lock) = DAEMON_TX.get() {
        if let Ok(guard) = lock.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(ev);
            }
        }
    }
}

unsafe extern "system" fn service_ctrl_handler(
    dw_control: u32,
    dw_event_type: u32,
    _lp_event_data: *mut c_void,
    _lp_context: *mut c_void,
) -> u32 {
    match dw_control {
        SERVICE_CONTROL_STOP => {
            set_scm_state(SERVICE_STOP_PENDING.0, 0, 0, 5000);
            send_daemon_event(DaemonEvent::Stop);
            0
        }
        SERVICE_CONTROL_PRESHUTDOWN => {
            set_scm_state(SERVICE_STOP_PENDING.0, 0, 0, 5000);
            let (ack_tx, ack_rx) = mpsc::channel();
            send_daemon_event(DaemonEvent::Shutdown(ack_tx));
            let _ = ack_rx.recv_timeout(Duration::from_secs(5));
            0
        }
        SERVICE_CONTROL_POWEREVENT => {
            match dw_event_type {
                PBT_APMSUSPEND => {
                    let (ack_tx, ack_rx) = mpsc::channel();
                    send_daemon_event(DaemonEvent::Suspend(ack_tx));
                    let _ = ack_rx.recv_timeout(Duration::from_secs(5));
                }
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                    send_daemon_event(DaemonEvent::Resume);
                }
                _ => {}
            }
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => WIN32_ERROR_CALL_NOT_IMPLEMENTED,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let svc_name = HSTRING::from(SERVICE_NAME);
    let handle = match unsafe {
        RegisterServiceCtrlHandlerExW(PCWSTR(svc_name.as_ptr()), Some(service_ctrl_handler), None)
    } {
        Ok(h) => h,
        Err(_) => {
            daemon_log("RegisterServiceCtrlHandlerExW failed");
            return;
        }
    };

    let _ = SVC_STATUS_HANDLE.set(handle.0 as usize);
    set_scm_state(SERVICE_START_PENDING.0, 0, 0, 3000);

    let (tx, rx) = mpsc::channel::<DaemonEvent>();
    let lock = DAEMON_TX.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(tx);
    }

    let accepted = SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PRESHUTDOWN | SERVICE_ACCEPT_POWEREVENT;
    set_scm_state(SERVICE_RUNNING.0, accepted, 0, 0);

    let code = run_daemon_loop(&rx);

    if let Ok(mut guard) = lock.lock() {
        *guard = None;
    }
    set_scm_state(SERVICE_STOPPED.0, 0, code, 0);
}

fn run_daemon_loop(rx: &Receiver<DaemonEvent>) -> u32 {
    let _wmi_scope = HpWmiBios::enable_thread_cache();
    let battery_be = WindowsBatteryBackend::new();
    let mut engine = BatteryCareEngine::new();

    if let Err(err) = ensure_secure_program_data_dir(&data_dir()) {
        daemon_log(&format!("Warning: ProgramData ACL repair returned: {err}"));
    }
    let cfg_path = config_file_path();
    let mut svc_cfg = BatteryServiceConfig::load_from_path(&cfg_path).unwrap_or_default();
    let mut bat_cfg = svc_cfg.to_battery_config();
    let mut last_cfg_mtime = fs::metadata(&cfg_path).and_then(|m| m.modified()).ok();
    let mut last_applied: Option<BatteryChargeMode> = None;
    let mut last_eval = Instant::now() - Duration::from_secs(60);

    daemon_log(&format!(
        "OmvixBatteryService started (enabled={}, stop={}%, start={}%, mode={:?}, hold_when_shutdown={})",
        svc_cfg.enabled,
        svc_cfg.stop_threshold_pct,
        svc_cfg.start_threshold_pct,
        svc_cfg.mode,
        svc_cfg.hold_when_shutdown
    ));

    loop {
        // Reload config promptly when updated from the Omvix Hub UI
        let cur_mtime = fs::metadata(&cfg_path).and_then(|m| m.modified()).ok();
        if cur_mtime.is_some() && cur_mtime != last_cfg_mtime {
            last_cfg_mtime = cur_mtime;
            if is_reparse_point(&cfg_path) {
                daemon_log("Refusing config reload: target path is a reparse point");
            } else if let Ok(reloaded) = BatteryServiceConfig::load_from_path(&cfg_path) {
                svc_cfg = reloaded;
                bat_cfg = svc_cfg.to_battery_config();
                last_applied = None;
                last_eval = Instant::now() - Duration::from_secs(60);
                daemon_log(&format!(
                    "Config reloaded (enabled={}, stop={}%, start={}%, mode={:?}, hold_when_shutdown={})",
                    svc_cfg.enabled,
                    svc_cfg.stop_threshold_pct,
                    svc_cfg.start_threshold_pct,
                    svc_cfg.mode,
                    svc_cfg.hold_when_shutdown
                ));
            }
        }

        let poll_secs = svc_cfg.poll_interval_secs.clamp(2, 60);
        if last_eval.elapsed() >= Duration::from_secs(poll_secs) {
            last_eval = Instant::now();
            match battery_be.get_battery_status() {
                Ok(bat) => {
                    let target = if bat_cfg.enabled {
                        engine.evaluate(&mut bat_cfg, bat.charge_percent, bat.ac_connected)
                    } else {
                        BatteryChargeMode::Auto
                    };
                    if last_applied != Some(target) {
                        match battery_be.set_charge_mode(target) {
                            Ok(()) => {
                                last_applied = Some(target);
                                daemon_log(&format!(
                                    "Applied {target:?} (bat={}%, ac={})",
                                    bat.charge_percent, bat.ac_connected
                                ));
                            }
                            Err(err) => {
                                daemon_log(&format!("Failed setting {target:?}: {err}"));
                            }
                        }
                    }
                }
                Err(err) => {
                    daemon_log(&format!("Battery status read failed: {err}"));
                }
            }
        }

        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(DaemonEvent::Stop) => {
                if let Err(err) = battery_be.set_charge_mode(BatteryChargeMode::Auto) {
                    daemon_log(&format!("Stop Auto restore failed: {err}"));
                } else {
                    daemon_log("Service stopped -> restored 0x2B Auto");
                }
                return 0;
            }
            Ok(DaemonEvent::Shutdown(ack)) => {
                let target = match battery_be.get_battery_status() {
                    Ok(bat) => decide_off_state_mode(&svc_cfg, bat.charge_percent),
                    Err(_) => {
                        if svc_cfg.enabled && svc_cfg.hold_when_shutdown {
                            BatteryChargeMode::Hold
                        } else {
                            BatteryChargeMode::Auto
                        }
                    }
                };
                let _ = battery_be.set_charge_mode(target);
                daemon_log(&format!(
                    "System shutdown/preshutdown -> applied {target:?}"
                ));
                let _ = ack.send(());
                return 0;
            }
            Ok(DaemonEvent::Suspend(ack)) => {
                let target = match battery_be.get_battery_status() {
                    Ok(bat) => decide_off_state_mode(&svc_cfg, bat.charge_percent),
                    Err(_) => {
                        if svc_cfg.enabled && svc_cfg.hold_when_shutdown {
                            BatteryChargeMode::Hold
                        } else {
                            BatteryChargeMode::Auto
                        }
                    }
                };
                let _ = battery_be.set_charge_mode(target);
                daemon_log(&format!("System suspend/hibernate -> applied {target:?}"));
                let _ = ack.send(());
            }
            Ok(DaemonEvent::Resume) => {
                last_applied = None;
                last_eval = Instant::now() - Duration::from_secs(60);
                daemon_log("System resumed -> forcing immediate charge check");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return 0,
        }
    }
}

pub fn run_service_dispatcher() -> Result<()> {
    let mut name_utf16: Vec<u16> = SERVICE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name_utf16.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }
        .map_err(|e| OmvixError::WmiUnavailable(format!("StartServiceCtrlDispatcherW failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_command_line_quotes_path_and_adds_flag() {
        let cmd = service_command_line(Path::new(
            r"C:\Program Files\OmvixHub\omvix-battery-svc.exe",
        ));
        assert_eq!(
            cmd,
            r#""C:\Program Files\OmvixHub\omvix-battery-svc.exe" --battery-service"#
        );
    }

    #[test]
    fn battery_service_config_toml_roundtrip() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_svc_cfg_{nanos}"));
        let path = dir.join("battery-service.toml");

        let cfg = BatteryServiceConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 60,
            start_threshold_pct: 55,
            hold_when_shutdown: true,
            off_min_pct: 40,
            one_time_full_charge: false,
            poll_interval_secs: 5,
        };
        cfg.save_to_path(&path).expect("save service config");
        let loaded = BatteryServiceConfig::load_from_path(&path).expect("load service config");
        assert_eq!(loaded, cfg);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn decide_off_state_mode_respects_hold_and_low_battery_floor() {
        let mut cfg = BatteryServiceConfig {
            enabled: true,
            mode: BatteryChargeMode::Inhibit,
            stop_threshold_pct: 80,
            start_threshold_pct: 75,
            hold_when_shutdown: true,
            off_min_pct: 40,
            one_time_full_charge: false,
            poll_interval_secs: 5,
        };
        // Above start threshold & 40% floor -> Hold
        assert_eq!(decide_off_state_mode(&cfg, 80), BatteryChargeMode::Hold);
        assert_eq!(decide_off_state_mode(&cfg, 76), BatteryChargeMode::Hold);
        // Below start threshold (e.g. 60% < 75%) or low battery -> Auto
        assert_eq!(decide_off_state_mode(&cfg, 60), BatteryChargeMode::Auto);
        assert_eq!(decide_off_state_mode(&cfg, 25), BatteryChargeMode::Auto);

        // Travel bypass active -> Auto
        cfg.one_time_full_charge = true;
        assert_eq!(decide_off_state_mode(&cfg, 85), BatteryChargeMode::Auto);

        // hold_when_shutdown disabled -> Auto
        cfg.one_time_full_charge = false;
        cfg.hold_when_shutdown = false;
        assert_eq!(decide_off_state_mode(&cfg, 85), BatteryChargeMode::Auto);

        // Limiter disabled -> Auto
        cfg.hold_when_shutdown = true;
        cfg.enabled = false;
        assert_eq!(decide_off_state_mode(&cfg, 85), BatteryChargeMode::Auto);
    }
}
