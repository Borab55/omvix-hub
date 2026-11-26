use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Power::{
    RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassW, TranslateMessage, UnregisterClassW,
    DEVICE_NOTIFY_WINDOW_HANDLE, HMENU, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL,
    PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, WINDOW_EX_STYLE, WM_ENDSESSION, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_QUIT, WNDCLASSW, WS_OVERLAPPED,
};

const POWER_CLASS_NAME: &str = "OmvixPowerEventWindow";
const RESUME_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPowerEvent {
    Suspending,
    Resumed,
    ShuttingDown,
}

pub struct PowerEventHandle {
    thread_id: Option<u32>,
    join: Option<JoinHandle<()>>,
}

impl PowerEventHandle {
    pub fn stop(mut self) {
        self.signal_and_join();
    }

    fn signal_and_join(&mut self) {
        if let Some(tid) = self.thread_id.take() {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PowerEventHandle {
    fn drop(&mut self) {
        self.signal_and_join();
    }
}

type PowerCallback = Arc<dyn Fn(SystemPowerEvent) + Send + Sync + 'static>;

static POWER_CALLBACK: OnceLock<Mutex<Option<PowerCallback>>> = OnceLock::new();
static LAST_RESUME_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

pub fn classify_power_message(msg: u32, wparam: usize) -> Option<SystemPowerEvent> {
    if msg == WM_POWERBROADCAST {
        match wparam as u32 {
            PBT_APMSUSPEND => Some(SystemPowerEvent::Suspending),
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                Some(SystemPowerEvent::Resumed)
            }
            _ => None,
        }
    } else if msg == WM_ENDSESSION && wparam != 0 {
        Some(SystemPowerEvent::ShuttingDown)
    } else {
        None
    }
}

fn should_dispatch_event(ev: SystemPowerEvent) -> bool {
    if ev != SystemPowerEvent::Resumed {
        return true;
    }
    let lock = LAST_RESUME_AT.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = lock.lock() else {
        return true;
    };
    if guard.is_some_and(|prev| prev.elapsed() < RESUME_DEBOUNCE) {
        return false;
    }
    *guard = Some(Instant::now());
    true
}

fn dispatch_power_event(ev: SystemPowerEvent) {
    if !should_dispatch_event(ev) {
        return;
    }
    let Some(lock) = POWER_CALLBACK.get() else {
        return;
    };
    let cb = lock.lock().ok().and_then(|guard| guard.clone());
    if let Some(cb) = cb {
        cb(ev);
    }
}

unsafe extern "system" fn power_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_QUERYENDSESSION {
        return LRESULT(1);
    }

    if let Some(ev) = classify_power_message(msg, wparam.0) {
        dispatch_power_event(ev);
        return if msg == WM_POWERBROADCAST {
            LRESULT(1)
        } else {
            LRESULT(0)
        };
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Spawns a hidden top-level Win32 message window listening for Sleep, Modern Standby (S0ix),
/// Hibernate, Resume, and Windows Shutdown/Logoff broadcasts.
pub fn spawn_power_event_listener<F>(on_event: F) -> PowerEventHandle
where
    F: Fn(SystemPowerEvent) + Send + Sync + 'static,
{
    let cb_slot = POWER_CALLBACK.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cb_slot.lock() {
        *guard = Some(Arc::new(on_event));
    }

    let (tid_tx, tid_rx) = mpsc::channel::<Option<u32>>();
    let join = thread::spawn(move || {
        let tid = unsafe { GetCurrentThreadId() };

        let class_utf16: Vec<u16> = POWER_CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let title_utf16: Vec<u16> = "OmvixPowerListener"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(power_wnd_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: PCWSTR(class_utf16.as_ptr()),
            ..Default::default()
        };

        let atom = unsafe { RegisterClassW(&wc) };
        if atom == 0 {
            tracing::debug!("RegisterClassW for OmvixPowerEventWindow returned 0");
        }

        let hwnd = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_utf16.as_ptr()),
                PCWSTR(title_utf16.as_ptr()),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                HWND::default(),
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
        } {
            Ok(h) => h,
            Err(err) => {
                tracing::warn!("Could not create power event listener window: {err}");
                let _ = tid_tx.send(None);
                return;
            }
        };

        let _ = tid_tx.send(Some(tid));

        let notify_handle = unsafe {
            RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
        }
        .ok();
        if notify_handle.is_none() {
            tracing::debug!(
                "RegisterSuspendResumeNotification failed; relying on WM_POWERBROADCAST"
            );
        } else {
            tracing::info!("Started Windows Sleep / Modern Standby / Shutdown listener");
        }

        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if let Some(nh) = notify_handle {
            unsafe {
                let _ = UnregisterSuspendResumeNotification(nh);
            }
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(PCWSTR(class_utf16.as_ptr()), HINSTANCE::default());
        }
        if let Some(lock) = POWER_CALLBACK.get() {
            if let Ok(mut guard) = lock.lock() {
                *guard = None;
            }
        }
        tracing::debug!("Stopped Windows power event listener");
    });

    let thread_id = tid_rx
        .recv_timeout(Duration::from_millis(500))
        .ok()
        .flatten();
    PowerEventHandle {
        thread_id,
        join: Some(join),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_suspend_resume_and_shutdown_messages() {
        assert_eq!(
            classify_power_message(WM_POWERBROADCAST, PBT_APMSUSPEND as usize),
            Some(SystemPowerEvent::Suspending)
        );
        assert_eq!(
            classify_power_message(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC as usize),
            Some(SystemPowerEvent::Resumed)
        );
        assert_eq!(
            classify_power_message(WM_POWERBROADCAST, PBT_APMRESUMESUSPEND as usize),
            Some(SystemPowerEvent::Resumed)
        );
        assert_eq!(
            classify_power_message(WM_POWERBROADCAST, PBT_APMRESUMECRITICAL as usize),
            Some(SystemPowerEvent::Resumed)
        );
        assert_eq!(
            classify_power_message(WM_ENDSESSION, 1),
            Some(SystemPowerEvent::ShuttingDown)
        );
        assert_eq!(classify_power_message(WM_ENDSESSION, 0), None);
        assert_eq!(classify_power_message(WM_QUERYENDSESSION, 1), None);
        assert_eq!(classify_power_message(0, 0), None);
    }
}
