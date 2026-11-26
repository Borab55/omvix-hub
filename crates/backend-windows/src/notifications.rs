use std::mem::size_of;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, SetWindowRgn,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_LEFT, DT_SINGLELINE,
    DT_VCENTER, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    TRANSPARENT as GDI_TRANSPARENT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_INFO, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, KillTimer, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes,
    SetTimer, ShowWindow, TranslateMessage, HMENU, HWND_MESSAGE, LWA_ALPHA, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_PAINT, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

const OSD_CLASS_NAME: &str = "OmvixHubOsdClass";
const OSD_WIDTH: i32 = 320;
const OSD_HEIGHT: i32 = 72;
const OSD_TIMER_ID: usize = 1;
const OSD_DURATION_MS: u64 = 1700;
const TOAST_DURATION_MS: u64 = 4000;
const TOAST_ICON_ID: u32 = 0x4F56_0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
struct OsdState {
    title: String,
    subtitle: String,
    accent_rgb: [u8; 3],
    expires_at: Instant,
    hwnd_raw: isize,
    spawning: bool,
}

static OSD_STATE: OnceLock<Mutex<OsdState>> = OnceLock::new();
static OSD_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

fn osd_state() -> &'static Mutex<OsdState> {
    OSD_STATE.get_or_init(|| {
        Mutex::new(OsdState {
            title: String::new(),
            subtitle: String::new(),
            accent_rgb: [0, 196, 235],
            expires_at: Instant::now(),
            hwnd_raw: 0,
            spawning: false,
        })
    })
}

/// Encodes `text` as a fixed-size NUL-terminated UTF-16 buffer for Win32 structs.
pub fn encode_utf16_fixed<const N: usize>(text: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    if N == 0 {
        return buf;
    }
    for (dst, ch) in buf[..N - 1].iter_mut().zip(text.encode_utf16()) {
        *dst = ch;
    }
    buf
}

/// Converts an `[r, g, b]` triplet into a Win32 GDI `COLORREF` (`0x00BBGGRR`).
pub fn rgb_to_colorref(rgb: [u8; 3]) -> u32 {
    u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16)
}

/// Sends a native Windows Notification Center balloon/toast message without blocking the caller.
pub fn send_windows_toast(title: &str, body: &str, severity: ToastSeverity) {
    let title = title.to_string();
    let body = body.to_string();

    thread::spawn(move || {
        let class_name = HSTRING::from("STATIC");
        let empty = HSTRING::from("");
        let Ok(hwnd) = (unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(empty.as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
        }) else {
            return;
        };

        let nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TOAST_ICON_ID,
            uFlags: NIF_INFO | NIF_TIP,
            szTip: encode_utf16_fixed("Omvix Hub"),
            szInfoTitle: encode_utf16_fixed(&title),
            szInfo: encode_utf16_fixed(&body),
            dwInfoFlags: match severity {
                ToastSeverity::Info => NIIF_INFO,
                ToastSeverity::Warning => NIIF_WARNING,
                ToastSeverity::Error => NIIF_ERROR,
            },
            ..Default::default()
        };

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) }.as_bool();
        if added {
            thread::sleep(Duration::from_millis(TOAST_DURATION_MS));
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    });
}

unsafe extern "system" fn osd_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == OSD_TIMER_ID => {
            let expired = osd_state()
                .lock()
                .map(|s| Instant::now() >= s.expires_at)
                .unwrap_or(true);
            if expired {
                unsafe {
                    let _ = KillTimer(hwnd, OSD_TIMER_ID);
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let snapshot = osd_state().lock().ok().map(|s| s.clone());
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                let bg_brush = unsafe { CreateSolidBrush(COLORREF(rgb_to_colorref([22, 24, 30]))) };
                let full_rect = RECT {
                    left: 0,
                    top: 0,
                    right: OSD_WIDTH,
                    bottom: OSD_HEIGHT,
                };
                unsafe {
                    let _ = FillRect(hdc, &full_rect, bg_brush);
                    let _ = DeleteObject(bg_brush);
                }

                if let Some(st) = snapshot {
                    let accent_brush =
                        unsafe { CreateSolidBrush(COLORREF(rgb_to_colorref(st.accent_rgb))) };
                    let strip_rect = RECT {
                        left: 0,
                        top: 0,
                        right: 6,
                        bottom: OSD_HEIGHT,
                    };
                    unsafe {
                        let _ = FillRect(hdc, &strip_rect, accent_brush);
                        let _ = DeleteObject(accent_brush);
                        let _ = SetBkMode(hdc, GDI_TRANSPARENT);
                    }

                    let face = HSTRING::from("Segoe UI");
                    let title_font = unsafe {
                        CreateFontW(
                            20,
                            0,
                            0,
                            0,
                            FW_SEMIBOLD.0 as i32,
                            0,
                            0,
                            0,
                            u32::from(DEFAULT_CHARSET.0),
                            u32::from(OUT_DEFAULT_PRECIS.0),
                            u32::from(CLIP_DEFAULT_PRECIS.0),
                            u32::from(CLEARTYPE_QUALITY.0),
                            u32::from(DEFAULT_PITCH.0) | u32::from(FF_DONTCARE.0),
                            PCWSTR(face.as_ptr()),
                        )
                    };
                    let sub_font = unsafe {
                        CreateFontW(
                            15,
                            0,
                            0,
                            0,
                            FW_NORMAL.0 as i32,
                            0,
                            0,
                            0,
                            u32::from(DEFAULT_CHARSET.0),
                            u32::from(OUT_DEFAULT_PRECIS.0),
                            u32::from(CLIP_DEFAULT_PRECIS.0),
                            u32::from(CLEARTYPE_QUALITY.0),
                            u32::from(DEFAULT_PITCH.0) | u32::from(FF_DONTCARE.0),
                            PCWSTR(face.as_ptr()),
                        )
                    };

                    let old_font = unsafe { SelectObject(hdc, title_font) };
                    let mut title_rect = RECT {
                        left: 18,
                        top: 12,
                        right: OSD_WIDTH - 14,
                        bottom: 38,
                    };
                    let mut title_u16: Vec<u16> = st.title.encode_utf16().collect();
                    unsafe {
                        let _ = SetTextColor(hdc, COLORREF(rgb_to_colorref([242, 245, 250])));
                        let _ = DrawTextW(
                            hdc,
                            &mut title_u16,
                            &mut title_rect,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                        );
                    }

                    unsafe {
                        let _ = SelectObject(hdc, sub_font);
                        let _ = SetTextColor(hdc, COLORREF(rgb_to_colorref([176, 184, 198])));
                    }
                    let mut sub_rect = RECT {
                        left: 18,
                        top: 38,
                        right: OSD_WIDTH - 14,
                        bottom: 62,
                    };
                    let mut sub_u16: Vec<u16> = st.subtitle.encode_utf16().collect();
                    unsafe {
                        let _ = DrawTextW(
                            hdc,
                            &mut sub_u16,
                            &mut sub_rect,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                        );
                        let _ = SelectObject(hdc, old_font);
                        let _ = DeleteObject(title_font);
                        let _ = DeleteObject(sub_font);
                    }
                }

                unsafe {
                    let _ = EndPaint(hwnd, &ps);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Ok(mut st) = osd_state().lock() {
                if st.hwnd_raw == hwnd.0 as isize {
                    st.hwnd_raw = 0;
                    st.spawning = false;
                }
            }
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Displays a brief translucent click-through OSD badge in the lower screen area.
/// Rapid mode cycles update the existing popup cleanly without stacking windows.
pub fn show_mode_osd(title: &str, subtitle: &str, accent_rgb: [u8; 3]) {
    let (existing_hwnd, should_spawn) = {
        let Ok(mut st) = osd_state().lock() else {
            return;
        };
        st.title = title.to_string();
        st.subtitle = subtitle.to_string();
        st.accent_rgb = accent_rgb;
        st.expires_at = Instant::now() + Duration::from_millis(OSD_DURATION_MS);
        let need_spawn = st.hwnd_raw == 0 && !st.spawning;
        if need_spawn {
            st.spawning = true;
        }
        (st.hwnd_raw, need_spawn)
    };

    if existing_hwnd != 0 {
        let hwnd = HWND(existing_hwnd as *mut std::ffi::c_void);
        unsafe {
            let _ = InvalidateRect(hwnd, None, true);
        }
        return;
    }
    if !should_spawn {
        return;
    }

    thread::spawn(move || {
        let class_name = HSTRING::from(OSD_CLASS_NAME);
        let empty = HSTRING::from("");
        OSD_CLASS_REGISTERED.get_or_init(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(osd_wnd_proc),
                hInstance: HINSTANCE::default(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            unsafe {
                let _ = RegisterClassW(&wc);
            }
        });

        let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(OSD_WIDTH + 32);
        let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(OSD_HEIGHT + 160);
        let x = (sw - OSD_WIDTH) / 2;
        let y = (sh * 4 / 5).min(sh - OSD_HEIGHT - 48);

        let ex_style =
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED;
        let Ok(hwnd) = (unsafe {
            CreateWindowExW(
                ex_style,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(empty.as_ptr()),
                WS_POPUP,
                x,
                y,
                OSD_WIDTH,
                OSD_HEIGHT,
                HWND::default(),
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
        }) else {
            if let Ok(mut st) = osd_state().lock() {
                st.spawning = false;
            }
            return;
        };

        unsafe {
            let rgn = CreateRoundRectRgn(0, 0, OSD_WIDTH + 1, OSD_HEIGHT + 1, 20, 20);
            if !rgn.is_invalid() && SetWindowRgn(hwnd, rgn, true) == 0 {
                let _ = DeleteObject(rgn);
            }
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 232, LWA_ALPHA);
        }

        if let Ok(mut st) = osd_state().lock() {
            st.hwnd_raw = hwnd.0 as isize;
            st.spawning = false;
        }

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = InvalidateRect(hwnd, None, true);
            let _ = SetTimer(hwnd, OSD_TIMER_ID, 80, None);
        }

        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_utf16_fixed_truncates_and_nul_terminates() {
        let exact: [u16; 6] = encode_utf16_fixed("Omvix");
        assert_eq!(
            exact,
            [
                b'O' as u16,
                b'm' as u16,
                b'v' as u16,
                b'i' as u16,
                b'x' as u16,
                0
            ]
        );

        let clipped: [u16; 4] = encode_utf16_fixed("Omvix Hub");
        assert_eq!(clipped, [b'O' as u16, b'm' as u16, b'v' as u16, 0]);
    }

    #[test]
    fn rgb_to_colorref_packs_bgr_correctly() {
        assert_eq!(rgb_to_colorref([0x12, 0x34, 0x56]), 0x0056_3412);
        assert_eq!(rgb_to_colorref([255, 0, 128]), 0x0080_00FF);
    }
}
