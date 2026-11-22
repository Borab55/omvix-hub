use crate::wmi_com::{get_u32, WmiConnection};
use omvix_core::{OmvixError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Wmi::{IWbemClassObject, WBEM_E_ACCESS_DENIED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT,
};

const OMEN_KEY_EVENT_DATA: u32 = 8613; // 0x21A5
pub const WIN32_ERR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyBindError {
    InvalidSyntax(String),
    Conflict { shortcut: String, os_code: u32 },
    OsError { shortcut: String, message: String },
}

impl std::fmt::Display for HotkeyBindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSyntax(msg) => write!(f, "Invalid shortcut syntax: {msg}"),
            Self::Conflict { shortcut, os_code } => write!(
                f,
                "Shortcut `{shortcut}` is already registered by another app or Windows (Win32 error {os_code})"
            ),
            Self::OsError { shortcut, message } => {
                write!(f, "Failed registering shortcut `{shortcut}`: {message}")
            }
        }
    }
}

impl std::error::Error for HotkeyBindError {}

pub fn classify_register_hotkey_hresult(
    shortcut: &str,
    hresult_code: i32,
    message: String,
) -> HotkeyBindError {
    let win32_code = (hresult_code as u32) & 0xFFFF;
    if win32_code == WIN32_ERR_HOTKEY_ALREADY_REGISTERED {
        HotkeyBindError::Conflict {
            shortcut: shortcut.to_string(),
            os_code: win32_code,
        }
    } else {
        HotkeyBindError::OsError {
            shortcut: shortcut.to_string(),
            message,
        }
    }
}

/// Parses a human-readable hotkey specification such as `"Ctrl+Shift+O"` or `"Alt+F12"`.
pub fn parse_shortcut(spec: &str) -> Result<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut vk: Option<u32> = None;

    for raw_part in spec.split('+') {
        let token = raw_part.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "alt" => mods |= MOD_ALT,
            "win" | "super" => mods |= MOD_WIN,
            other => {
                vk = Some(parse_virtual_key(other).ok_or_else(|| {
                    OmvixError::Config(format!("Unsupported hotkey key token: `{token}`"))
                })?);
            }
        }
    }

    let key = vk.ok_or_else(|| OmvixError::Config(format!("Shortcut `{spec}` missing key")))?;
    Ok((mods, key))
}

fn parse_virtual_key(token: &str) -> Option<u32> {
    if token.len() == 1 {
        let ch = token.bytes().next()?.to_ascii_uppercase();
        if ch.is_ascii_alphanumeric() {
            return Some(ch as u32);
        }
    }
    if let Some(rest) = token.strip_prefix('f') {
        if let Ok(num) = rest.parse::<u32>() {
            if (1..=24).contains(&num) {
                return Some(0x70 + (num - 1)); // VK_F1..VK_F24
            }
        }
    }
    match token {
        "space" => Some(0x20),
        "tab" => Some(0x09),
        "esc" | "escape" => Some(0x1B),
        "enter" | "return" => Some(0x0D),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "insert" => Some(0x2D),
        "delete" | "del" => Some(0x2E),
        _ => None,
    }
}

pub struct ListenerHandle {
    stop: Arc<AtomicBool>,
    thread_id: Option<u32>,
    join: Option<JoinHandle<()>>,
}

impl ListenerHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tid) = self.thread_id {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tid) = self.thread_id {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Subscribes to `root\WMI` `hpqBEvnt` notifications for the dedicated HP Omen key (`8613`).
pub fn spawn_omen_key_listener<F>(on_press: F) -> ListenerHandle
where
    F: Fn() + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let join = thread::spawn(move || {
        let Ok(wmi) = WmiConnection::connect(r"root\WMI") else {
            tracing::warn!("Omen key listener could not connect to root\\WMI");
            return;
        };
        let Ok(enumerator) = wmi.exec_notification_query("SELECT * FROM hpqBEvnt") else {
            tracing::info!("Omen key WMI subscription unavailable (requires Admin or unsupported)");
            return;
        };

        tracing::info!("Started WMI hpqBEvnt Omen key listener");
        while !stop_flag.load(Ordering::SeqCst) {
            let mut row: [Option<IWbemClassObject>; 1] = [None];
            let mut returned = 0u32;
            let hr = unsafe { enumerator.Next(1000, &mut row, &mut returned) };
            if hr.0 == WBEM_E_ACCESS_DENIED.0 {
                tracing::warn!("Omen key listener stopped due to WMI access denied");
                break;
            }
            if returned > 0 {
                if let Some(obj) = row[0].take() {
                    let event_id = get_u32(&obj, "EventID").unwrap_or(0);
                    let event_data = get_u32(&obj, "EventData").unwrap_or(0);
                    tracing::debug!(event_id, event_data, "Received hpqBEvnt WMI event");
                    if event_data == OMEN_KEY_EVENT_DATA {
                        on_press();
                    }
                }
            }
        }
    });

    ListenerHandle {
        stop,
        thread_id: None,
        join: Some(join),
    }
}

/// Registers a global Windows hotkey on a dedicated message-loop thread, classifying Win32 conflicts (`1409`).
pub fn spawn_global_hotkey_listener<F>(
    spec: &str,
    on_trigger: F,
) -> std::result::Result<ListenerHandle, HotkeyBindError>
where
    F: Fn() + Send + 'static,
{
    let (mods, vk) =
        parse_shortcut(spec).map_err(|e| HotkeyBindError::InvalidSyntax(e.to_string()))?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let (tid_tx, tid_rx) = std::sync::mpsc::channel::<std::result::Result<u32, HotkeyBindError>>();
    let spec_owned = spec.to_string();

    let join = thread::spawn(move || {
        let tid = unsafe { GetCurrentThreadId() };
        let reg_res = unsafe { RegisterHotKey(HWND::default(), 1, mods, vk) };
        if let Err(err) = reg_res {
            let classified =
                classify_register_hotkey_hresult(&spec_owned, err.code().0, err.to_string());
            let _ = tid_tx.send(Err(classified));
            return;
        }
        let _ = tid_tx.send(Ok(tid));
        tracing::info!(shortcut = %spec_owned, "Registered global toggle hotkey");

        let mut msg = MSG::default();
        while !stop_flag.load(Ordering::SeqCst) {
            let got = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
            if got.0 <= 0 {
                break;
            }
            if msg.message == WM_HOTKEY {
                on_trigger();
            }
        }

        unsafe {
            let _ = UnregisterHotKey(HWND::default(), 1);
        }
    });

    let tid = tid_rx.recv().map_err(|e| HotkeyBindError::OsError {
        shortcut: spec.to_string(),
        message: format!("Hotkey thread failed to start: {e}"),
    })??;

    Ok(ListenerHandle {
        stop,
        thread_id: Some(tid),
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_shortcuts() {
        let (mods, vk) = parse_shortcut("Ctrl+Shift+O").expect("valid Ctrl+Shift+O");
        assert_eq!(mods, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(vk, b'O' as u32);

        let (mods2, vk2) = parse_shortcut("Alt+F12").expect("valid Alt+F12");
        assert_eq!(mods2, MOD_ALT);
        assert_eq!(vk2, 0x70 + 11);

        assert!(parse_shortcut("Ctrl+").is_err());
        assert!(parse_shortcut("Ctrl+InvalidKeyName").is_err());
    }

    #[test]
    fn classifies_hotkey_conflict_and_other_os_errors() {
        let hresult_1409 = (0x8007_0000u32 | WIN32_ERR_HOTKEY_ALREADY_REGISTERED) as i32;
        assert_eq!(
            classify_register_hotkey_hresult(
                "Ctrl+Shift+O",
                hresult_1409,
                "Hot key is already registered.".to_string()
            ),
            HotkeyBindError::Conflict {
                shortcut: "Ctrl+Shift+O".to_string(),
                os_code: 1409,
            }
        );

        let other = (0x8007_0057u32) as i32;
        assert!(matches!(
            classify_register_hotkey_hresult("Ctrl+Shift+O", other, "Invalid param".into()),
            HotkeyBindError::OsError { .. }
        ));
    }
}
