#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod theme;
mod tray;
mod ui;
mod worker;

use crate::tray::{build_tray_icon_rgba, TrayAction, TrayController};
use crate::ui::{Drawer, ExtraSubTab, FansPowerSubTab};
use crate::worker::{BackgroundWorker, SharedState, WorkerCommand};
use egui::Margin;
use omvix_core::config::OmenKeyAction;
use omvix_core::crash_guard::{
    crash_guard_path_for, current_unix_secs, resolve_startup_safe_mode, CrashGuardState,
    SafeModeReason,
};
use omvix_core::{init_logging, AppConfig, CustomThemeConfig, I18n, LogHandle, PerformanceMode};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const COMPACT_SIZE: [f32; 2] = [468.0, 728.0];
const DRAWER_SIZE: [f32; 2] = [632.0, 728.0];

#[cfg(windows)]
use omvix_windows::hotkeys::{
    spawn_global_hotkey_listener, spawn_omen_key_listener, HotkeyBindError, ListenerHandle,
};
#[cfg(windows)]
use omvix_windows::power_events::{spawn_power_event_listener, PowerEventHandle, SystemPowerEvent};

#[derive(Debug, Clone)]
enum AppSignal {
    SyncVisibility(bool),
    TrayPerfChanged(PerformanceMode),
    TrayFanChanged(omvix_core::FanMode),
    TrayBatteryPreset {
        enabled: bool,
        stop_pct: u8,
        start_pct: u8,
    },
    OmenKeyNonToggle,
    QuitRequested,
}

fn perf_mode_label(i18n: &I18n, mode: PerformanceMode) -> String {
    match mode {
        PerformanceMode::Eco => i18n.t("thermal.mode_eco"),
        PerformanceMode::Quiet => i18n.t("thermal.mode_quiet"),
        PerformanceMode::Balanced => i18n.t("thermal.mode_balanced"),
        PerformanceMode::Performance => i18n.t("thermal.mode_performance"),
    }
}

fn fan_mode_label(i18n: &I18n, mode: omvix_core::FanMode) -> String {
    match mode {
        omvix_core::FanMode::Auto => i18n.t("fan.mode_auto"),
        omvix_core::FanMode::Max => i18n.t("fan.mode_max"),
        omvix_core::FanMode::Manual => i18n.t("fan.mode_manual"),
        omvix_core::FanMode::CustomCurve => i18n.t("fan.mode_curve"),
    }
}

fn show_mode_change_osd(title: &str, subtitle: &str, mode: PerformanceMode) {
    #[cfg(windows)]
    {
        let rgb = tray::performance_mode_ring_rgb(mode).0;
        omvix_windows::notifications::show_mode_osd(title, subtitle, rgb);
    }
    #[cfg(not(windows))]
    let _ = (title, subtitle, mode);
}

fn omen_action_to_code(action: &OmenKeyAction) -> u8 {
    match action {
        OmenKeyAction::ToggleWindow => 0,
        OmenKeyAction::CyclePerformanceMode => 1,
        OmenKeyAction::LaunchCustomCommand => 2,
    }
}

#[cfg(windows)]
struct SingleInstanceGuard(Option<isize>);

#[cfg(windows)]
impl SingleInstanceGuard {
    fn acquire() -> std::result::Result<Self, ()> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;

        let name = HSTRING::from("Local\\OmvixHubSingleInstance");
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(|_| ())?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
            return Err(());
        }
        Ok(Self(Some(handle.0 as isize)))
    }

    fn release(&mut self) {
        if let Some(raw) = self.0.take() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(
                    windows::Win32::Foundation::HANDLE(raw as *mut std::ffi::c_void),
                );
            }
        }
    }
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(windows)]
fn restore_existing_instance_window() {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };

    let title = HSTRING::from("Omvix Hub");
    if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) } {
        if !hwnd.is_invalid() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
fn restore_existing_instance_window() {}

#[cfg(windows)]
fn win32_set_visible(hwnd_raw: isize, visible: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
    };

    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    unsafe {
        if visible {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        } else {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
fn win32_set_visible(_hwnd_raw: isize, _visible: bool) {}

#[cfg(windows)]
fn win32_post_close(hwnd_raw: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    unsafe {
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(not(windows))]
fn win32_post_close(_hwnd_raw: isize) {}

#[cfg(windows)]
fn win32_visible_bounds(
    hwnd: windows::Win32::Foundation::HWND,
) -> Option<(
    windows::Win32::Foundation::RECT,
    windows::Win32::Foundation::RECT,
)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut wr = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut wr) }.is_err() {
        return None;
    }
    let mut vis = RECT::default();
    let dwm_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut vis as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .is_ok();
    if !dwm_ok || (vis.right - vis.left) <= 0 || (vis.bottom - vis.top) <= 0 {
        vis = wr;
    }
    Some((wr, vis))
}

#[cfg(windows)]
fn win32_dock_bottom_right(hwnd_raw: isize) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SystemParametersInfoW, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    let mut work = RECT::default();
    unsafe {
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut work as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
        {
            return;
        }
        let Some((wr, vis)) = win32_visible_bounds(hwnd) else {
            return;
        };
        let vis_w = vis.right - vis.left;
        let vis_h = vis.bottom - vis.top;
        if vis_w <= 0 || vis_h <= 0 {
            return;
        }
        // Compensate for invisible DWM shadow margin so visible frame aligns cleanly at bottom-right.
        let shadow_left = vis.left - wr.left;
        let shadow_top = vis.top - wr.top;
        let target_vis_x = (work.right - vis_w - 10).max(work.left + 8);
        let target_vis_y = (work.bottom - vis_h - 10).max(work.top + 8);
        let target_x = target_vis_x - shadow_left;
        let target_y = target_vis_y - shadow_top;
        if (wr.left - target_x).abs() <= 1 && (wr.top - target_y).abs() <= 1 {
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            target_x,
            target_y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
fn win32_dock_bottom_right(_hwnd_raw: isize) {}

#[cfg(windows)]
fn win32_drawer_anchor_pos(
    hwnd_raw: isize,
    drawer_w: f32,
    drawer_h: f32,
    ppp: f32,
) -> Option<egui::Pos2> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    if hwnd_raw == 0 || !ppp.is_finite() || ppp <= 0.0 {
        return None;
    }
    let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
    let mut work = RECT::default();
    let mut cr = RECT::default();
    unsafe {
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut work as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
        {
            return None;
        }
        let (wr, vis) = win32_visible_bounds(hwnd)?;
        if GetClientRect(hwnd, &mut cr).is_err() {
            return None;
        }
        let nc_w = ((wr.right - wr.left) - (cr.right - cr.left)).max(0);
        let nc_h = ((wr.bottom - wr.top) - (cr.bottom - cr.top)).max(0);
        let outer_w = (drawer_w * ppp).round() as i32 + nc_w;
        let outer_h = (drawer_h * ppp).round() as i32 + nc_h;
        let px_x = (vis.left - outer_w + (vis.left - wr.left) - 8).max(work.left + 8);
        let px_y = vis
            .top
            .clamp(work.top + 8, (work.bottom - outer_h - 8).max(work.top + 8));
        Some(egui::pos2(px_x as f32 / ppp, px_y as f32 / ppp))
    }
}

#[cfg(not(windows))]
fn win32_drawer_anchor_pos(
    _hwnd_raw: isize,
    _drawer_w: f32,
    _drawer_h: f32,
    _ppp: f32,
) -> Option<egui::Pos2> {
    None
}

#[cfg(windows)]
fn is_shift_held_at_startup() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
    let state = unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) };
    ((state as u16) & 0x8000) != 0
}

#[cfg(not(windows))]
fn is_shift_held_at_startup() -> bool {
    false
}

#[cfg(windows)]
fn try_shell_elevate_self(forward_safe: bool) -> std::result::Result<(), ()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let Ok(exe) = std::env::current_exe() else {
        return Err(());
    };
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe.to_string_lossy().as_ref());
    let params = HSTRING::from(if forward_safe {
        "--no-elevate --safe"
    } else {
        "--no-elevate"
    });
    let res = unsafe {
        ShellExecuteW(
            HWND::default(),
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if res.0 as isize > 32 {
        Ok(())
    } else {
        tracing::warn!("ShellExecuteW(runas) declined or failed: {:?}", res.0);
        Err(())
    }
}

#[cfg(windows)]
fn install_fan_safety_panic_hook(log_handle: Option<LogHandle>) {
    use std::sync::Once;
    static HOOK_ONCE: Once = Once::new();
    static PANIC_RECOVERY_RUNNING: AtomicBool = AtomicBool::new(false);

    HOOK_ONCE.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            previous_hook(panic_info);

            if !PANIC_RECOVERY_RUNNING.swap(true, Ordering::SeqCst) {
                let (tx, rx) = mpsc::channel();
                let _ = thread::spawn(move || {
                    let res = omvix_windows::thermal_fan::restore_fan_auto_fresh_wmi();
                    let _ = tx.send(res);
                });
                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(Ok(())) => {
                        tracing::warn!("Panic safety hook restored BIOS Auto fan control");
                    }
                    Ok(Err(err)) => {
                        tracing::error!("Panic safety hook failed to restore Auto fan: {err}");
                    }
                    Err(_) => {
                        tracing::error!(
                            "Panic safety hook timed out after 2s restoring BIOS Auto fan"
                        );
                    }
                }
                if let Some(ref h) = log_handle {
                    h.flush();
                }
                PANIC_RECOVERY_RUNNING.store(false, Ordering::SeqCst);
            }
        }));
    });
}

fn main() {
    #[cfg(windows)]
    omvix_windows::paths::harden_dll_search_path();

    let args: Vec<String> = std::env::args().collect();

    #[cfg(windows)]
    if args.iter().any(|a| a == "--battery-service") {
        if let Err(err) = omvix_windows::battery_service::run_service_dispatcher() {
            eprintln!("OmvixBatteryService error: {err}");
        }
        return;
    }

    let smoke_test = args.iter().any(|a| a == "--smoke-test");
    let cli_safe = args.iter().any(|a| a == "--safe");
    let shift_held = !smoke_test && is_shift_held_at_startup();
    #[cfg(windows)]
    let no_elevate = args.iter().any(|a| a == "--no-elevate");

    let cfg_path = AppConfig::default_config_path();
    let cfg_existed = cfg_path.exists();
    let outcome = AppConfig::load_or_quarantine_corrupt(&cfg_path);
    let mut cfg = outcome.config;
    let mut cfg_warnings = outcome.warnings;
    if cfg.general.theme == "light"
        && cfg.general.custom_theme == CustomThemeConfig::preset_dark_slate()
    {
        cfg.general.custom_theme = CustomThemeConfig::preset_clean_light();
    } else if cfg.general.custom_theme.is_legacy_bluish_dark_slate() {
        cfg.general.custom_theme = CustomThemeConfig::preset_dark_slate();
    }
    let log_handle = init_logging(&cfg.logging, smoke_test).ok();
    #[cfg(windows)]
    if !smoke_test {
        install_fan_safety_panic_hook(log_handle.clone());
    }
    for warning in &cfg_warnings {
        tracing::warn!("Config validation notice: {warning}");
    }
    let i18n = I18n::new(&cfg.general.language);
    if outcome.load_failed {
        let err_detail = outcome.load_error.as_deref().unwrap_or("parse error");
        if let Some(ref backup_path) = outcome.corrupt_backup {
            let backup_str = backup_path.display().to_string();
            cfg_warnings = vec![i18n.t_fmt(
                "config.corrupt_backup_notice",
                &[("path", &backup_str), ("error", err_detail)],
            )];
            if !smoke_test {
                let _ = cfg.save_to_path(&cfg_path);
            }
        } else if cfg_existed {
            cfg_warnings =
                vec![i18n.t_fmt("config.corrupt_rename_failed", &[("error", err_detail)])];
        }
    } else if !smoke_test && cfg_existed && !cfg_warnings.is_empty() {
        let _ = cfg.save_to_path(&cfg_path);
    }

    if smoke_test {
        let (rgba, w, h) = build_tray_icon_rgba();
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        tracing::info!(
            title = %i18n.t("app.title"),
            lang = %i18n.active_language(),
            "Omvix Hub smoke test passed"
        );
        if let Some(h) = log_handle {
            h.flush();
        }
        println!("{} smoke test OK", i18n.t("app.title"));
        return;
    }

    #[cfg(windows)]
    let single_instance = {
        let mut guard = match SingleInstanceGuard::acquire() {
            Ok(g) => g,
            Err(()) => {
                tracing::info!(
                    "Another Omvix Hub instance is already running; focusing existing window"
                );
                restore_existing_instance_window();
                if let Some(h) = &log_handle {
                    h.flush();
                }
                return;
            }
        };

        if !no_elevate && !omvix_windows::system_info::is_process_elevated() {
            guard.release();
            if try_shell_elevate_self(cli_safe || shift_held).is_ok() {
                tracing::info!("Launched elevated Omvix Hub instance; exiting non-admin launcher");
                if let Some(h) = &log_handle {
                    h.flush();
                }
                return;
            }
            tracing::info!(
                "UAC elevation declined on startup; continuing in non-admin telemetry mode"
            );
            SingleInstanceGuard::acquire().ok()
        } else {
            Some(guard)
        }
    };
    #[cfg(windows)]
    omvix_windows::updater::cleanup_stale_update_artifacts();

    let crash_guard_path = crash_guard_path_for(&cfg_path);
    let mut crash_guard_state = CrashGuardState::load_from_path(&crash_guard_path);
    let safe_mode_reason = resolve_startup_safe_mode(cli_safe, shift_held, &crash_guard_state);
    crash_guard_state.record_launch_start(current_unix_secs());
    if let Err(err) = crash_guard_state.save_to_path(&crash_guard_path) {
        tracing::warn!("Failed saving startup crash guard state: {err}");
    }
    if let Some(reason) = safe_mode_reason {
        tracing::warn!(
            ?reason,
            "Starting Omvix Hub in Safe Mode (all hardware writes disabled)"
        );
    }
    let crash_guard = Arc::new(Mutex::new(crash_guard_state));

    tracing::info!(lang = %i18n.active_language(), "Launching Omvix Hub desktop GUI");

    let start_visible = !cfg.general.start_minimized
        || safe_mode_reason.is_some()
        || !cfg.general.first_run_completed;
    let (rgba, icon_w, icon_h) = build_tray_icon_rgba();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Omvix Hub")
        .with_inner_size(COMPACT_SIZE)
        .with_min_inner_size([440.0, 580.0])
        .with_resizable(false)
        .with_maximize_button(false)
        .with_visible(start_visible)
        .with_icon(Arc::new(egui::IconData {
            rgba,
            width: icon_w,
            height: icon_h,
        }));

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    power_preference: eframe::wgpu::PowerPreference::LowPower,
                    ..Default::default()
                },
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    let tray_ctrl = match TrayController::new(&i18n) {
        Ok(t) => Some(t),
        Err(err) => {
            tracing::warn!("Could not initialize system tray icon: {err}");
            None
        }
    };

    let hwnd_atomic = Arc::new(AtomicIsize::new(0));
    let hwnd_setup = Arc::clone(&hwnd_atomic);

    if let Err(err) = eframe::run_native(
        "Omvix Hub",
        options,
        Box::new(move |cc| {
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                tracing::info!(
                    adapter = %info.name,
                    backend = ?info.backend,
                    device_type = ?info.device_type,
                    "Initialized wgpu graphics adapter"
                );
            }

            #[cfg(windows)]
            if let Ok(wh) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = wh.as_raw() {
                    hwnd_setup.store(h.hwnd.get(), Ordering::SeqCst);
                    tracing::debug!(hwnd = h.hwnd.get(), "Captured native Win32 HWND");
                }
            }

            theme::configure_fonts(&cc.egui_ctx);
            theme::apply_theme(
                &cc.egui_ctx,
                &cfg.general.custom_theme,
                cfg.general.font_scale_pct,
            );
            Ok(Box::new(OmvixApp::new(
                cc.egui_ctx.clone(),
                cfg,
                cfg_path,
                cfg_warnings,
                i18n,
                tray_ctrl,
                log_handle,
                hwnd_atomic,
                safe_mode_reason,
                crash_guard,
                crash_guard_path,
                #[cfg(windows)]
                single_instance,
            )))
        }),
    ) {
        tracing::error!("eframe exited with error: {err}");
    }
}

struct OmvixApp {
    cfg: AppConfig,
    cfg_path: PathBuf,
    config_warnings: Vec<String>,
    i18n: I18n,
    show_wizard: bool,
    wizard: ui::wizard::WizardState,
    open_drawer: Option<Drawer>,
    fans_power_tab: FansPowerSubTab,
    extra_tab: ExtraSubTab,
    pending_dock_frames: u8,
    last_shown_at: Instant,
    prev_focused: bool,
    config_dirty_since: Option<Instant>,
    worker_dead: bool,
    worker_dead_recovery_attempted: bool,
    performance_draft: ui::performance::PerformanceDraftState,
    profiles_draft: ui::profiles::ProfilesDraftState,
    battery_draft: ui::battery::BatteryDraftState,
    lighting_draft: ui::lighting_display::LightingDisplayDraftState,
    settings_draft: ui::settings::SettingsThemeDraft,
    hotkey_bind_status: ui::hotkeys::HotkeyBindStatus,
    last_seen_auto_revision: u64,
    last_seen_config_sync_rev: u64,
    window_visible: Arc<AtomicBool>,
    hwnd_atomic: Arc<AtomicIsize>,
    force_quit: Arc<AtomicBool>,
    clean_quit_started: Arc<AtomicBool>,
    omen_action_code: Arc<AtomicU8>,
    quitting: bool,
    tray: Option<TrayController>,
    worker: Option<BackgroundWorker>,
    state: Arc<Mutex<SharedState>>,
    signal_tx: Sender<AppSignal>,
    signal_rx: Receiver<AppSignal>,
    #[cfg(windows)]
    omen_listener: Option<ListenerHandle>,
    #[cfg(windows)]
    hotkey_listener: Option<ListenerHandle>,
    #[cfg(windows)]
    power_listener: Option<PowerEventHandle>,
    #[cfg(windows)]
    single_instance: Option<SingleInstanceGuard>,
    crash_guard: Arc<Mutex<CrashGuardState>>,
    crash_guard_path: PathBuf,
    log_handle: Option<LogHandle>,
}

impl OmvixApp {
    #[allow(clippy::too_many_arguments)]
    fn new(
        ctx: egui::Context,
        cfg: AppConfig,
        cfg_path: PathBuf,
        config_warnings: Vec<String>,
        i18n: I18n,
        tray: Option<TrayController>,
        log_handle: Option<LogHandle>,
        hwnd_atomic: Arc<AtomicIsize>,
        initial_safe_mode: Option<SafeModeReason>,
        crash_guard: Arc<Mutex<CrashGuardState>>,
        crash_guard_path: PathBuf,
        #[cfg(windows)] single_instance: Option<SingleInstanceGuard>,
    ) -> Self {
        let initial_visible = !cfg.general.start_minimized
            || initial_safe_mode.is_some()
            || !cfg.general.first_run_completed;
        let show_wizard = !cfg.general.first_run_completed && initial_safe_mode.is_none();
        let window_visible = Arc::new(AtomicBool::new(initial_visible));
        let force_quit = Arc::new(AtomicBool::new(false));
        let clean_quit_started = Arc::new(AtomicBool::new(false));
        let omen_action_code = Arc::new(AtomicU8::new(omen_action_to_code(
            &cfg.hotkeys.omen_key_action,
        )));

        let worker = BackgroundWorker::spawn(
            cfg.clone(),
            Some(ctx.clone()),
            Arc::clone(&window_visible),
            initial_safe_mode,
            Arc::clone(&crash_guard),
            crash_guard_path.clone(),
        );
        let state = worker.state();
        if !config_warnings.is_empty() {
            let joined = config_warnings.join(" | ");
            if let Ok(mut st) = state.lock() {
                st.set_toast(joined, true);
            }
        }
        let (signal_tx, signal_rx) = mpsc::channel::<AppSignal>();

        // Poll tray events every 80ms; handles show/hide/quit directly on Win32 HWND even when hidden.
        let tray_tx = signal_tx.clone();
        let wake_ctx = ctx.clone();
        let tray_vis = Arc::clone(&window_visible);
        let tray_hwnd = Arc::clone(&hwnd_atomic);
        let tray_force_quit = Arc::clone(&force_quit);
        let tray_quit_started = Arc::clone(&clean_quit_started);
        let tray_cfg_manual_speed = cfg.thermal_fan.manual_speed_pct;
        let _worker_clone_for_tray = worker_command_proxy(&worker);
        thread::spawn(move || loop {
            for action in TrayController::poll_actions() {
                match action {
                    TrayAction::ToggleWindow => {
                        let prev = tray_vis.fetch_xor(true, Ordering::SeqCst);
                        let next = !prev;
                        win32_set_visible(tray_hwnd.load(Ordering::SeqCst), next);
                        let _ = tray_tx.send(AppSignal::SyncVisibility(next));
                        wake_ctx.request_repaint();
                    }
                    TrayAction::SetPerformance(mode) => {
                        let _ =
                            _worker_clone_for_tray.send(WorkerCommand::SetPerformanceMode(mode));
                        let _ = tray_tx.send(AppSignal::TrayPerfChanged(mode));
                        wake_ctx.request_repaint();
                    }
                    TrayAction::SetFan(mode) => {
                        let _ = _worker_clone_for_tray.send(WorkerCommand::SetFanMode {
                            mode,
                            manual_speed_pct: Some(tray_cfg_manual_speed),
                        });
                        let _ = tray_tx.send(AppSignal::TrayFanChanged(mode));
                        wake_ctx.request_repaint();
                    }
                    TrayAction::ApplyBatteryPreset {
                        enabled,
                        stop_pct,
                        start_pct,
                    } => {
                        let _ = tray_tx.send(AppSignal::TrayBatteryPreset {
                            enabled,
                            stop_pct,
                            start_pct,
                        });
                        wake_ctx.request_repaint();
                    }
                    TrayAction::Quit => {
                        tracing::info!("Tray Quit action triggered");
                        tray_force_quit.store(true, Ordering::SeqCst);
                        let hwnd_raw = tray_hwnd.load(Ordering::SeqCst);
                        let _ = tray_tx.send(AppSignal::QuitRequested);
                        win32_post_close(hwnd_raw);
                        wake_ctx.request_repaint();

                        let quit_started_chk = Arc::clone(&tray_quit_started);
                        let wake_again = wake_ctx.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(250));
                            if !quit_started_chk.load(Ordering::SeqCst) {
                                // Nudge hidden winit event loop to process WM_CLOSE cleanly
                                win32_set_visible(hwnd_raw, true);
                                win32_post_close(hwnd_raw);
                                wake_again.request_repaint();
                                thread::sleep(Duration::from_millis(1500));
                            } else {
                                thread::sleep(Duration::from_millis(1500));
                            }
                            tracing::warn!("Tray quit watchdog forcing clean process termination");
                            std::process::exit(0);
                        });
                        return;
                    }
                }
            }
            thread::sleep(Duration::from_millis(80));
        });

        let dock_initial = if cfg.general.dock_bottom_right && initial_visible {
            2
        } else {
            0
        };
        let mut app = Self {
            cfg,
            cfg_path,
            config_warnings,
            i18n,
            show_wizard,
            wizard: ui::wizard::WizardState::default(),
            open_drawer: None,
            fans_power_tab: FansPowerSubTab::default(),
            extra_tab: ExtraSubTab::default(),
            pending_dock_frames: dock_initial,
            last_shown_at: Instant::now(),
            prev_focused: true,
            config_dirty_since: None,
            worker_dead: false,
            worker_dead_recovery_attempted: false,
            performance_draft: ui::performance::PerformanceDraftState::default(),
            profiles_draft: ui::profiles::ProfilesDraftState::default(),
            battery_draft: ui::battery::BatteryDraftState::default(),
            lighting_draft: ui::lighting_display::LightingDisplayDraftState::default(),
            settings_draft: ui::settings::SettingsThemeDraft::default(),
            hotkey_bind_status: ui::hotkeys::HotkeyBindStatus::Disabled,
            last_seen_auto_revision: 0,
            last_seen_config_sync_rev: 0,
            window_visible,
            hwnd_atomic,
            force_quit,
            clean_quit_started,
            omen_action_code,
            quitting: false,
            tray,
            worker: Some(worker),
            state,
            signal_tx,
            signal_rx,
            #[cfg(windows)]
            omen_listener: None,
            #[cfg(windows)]
            hotkey_listener: None,
            #[cfg(windows)]
            power_listener: None,
            #[cfg(windows)]
            single_instance,
            crash_guard,
            crash_guard_path,
            log_handle,
        };
        #[cfg(windows)]
        {
            if let Some(w) = &app.worker {
                let pwr_tx = worker_command_proxy(w);
                app.power_listener = Some(spawn_power_event_listener(move |ev| match ev {
                    SystemPowerEvent::Suspending => {
                        let (ack_tx, ack_rx) = mpsc::channel();
                        if pwr_tx.send(WorkerCommand::SystemSuspending(ack_tx)).is_ok()
                            && ack_rx.recv_timeout(Duration::from_secs(3)).is_err()
                        {
                            tracing::warn!("SystemSuspending safety hook timed out after 3s");
                        }
                    }
                    SystemPowerEvent::ShuttingDown => {
                        let (ack_tx, ack_rx) = mpsc::channel();
                        if pwr_tx
                            .send(WorkerCommand::SystemShuttingDown(ack_tx))
                            .is_ok()
                            && ack_rx.recv_timeout(Duration::from_secs(3)).is_err()
                        {
                            tracing::warn!("SystemShuttingDown safety hook timed out after 3s");
                        }
                    }
                    SystemPowerEvent::Resumed => {
                        let _ = pwr_tx.send(WorkerCommand::SystemResumed);
                    }
                }));
            }
        }
        app.rebind_hotkeys(&ctx);
        app
    }

    fn save_config(&mut self) {
        self.config_dirty_since = None;
        if let Err(err) = self.cfg.save_to_path(&self.cfg_path) {
            tracing::warn!("Failed to save config: {err}");
        }
    }

    fn flush_config_if_dirty(&mut self) {
        if self.config_dirty_since.is_some() {
            self.save_config();
        }
    }

    fn send_cmd(&self, cmd: WorkerCommand) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    fn stop_hotkeys(&mut self) {
        #[cfg(windows)]
        {
            if let Some(old) = self.omen_listener.take() {
                old.stop();
            }
            if let Some(old) = self.hotkey_listener.take() {
                old.stop();
            }
        }
    }

    /// Processes keyboard-only accessibility navigation (`Ctrl+1..4`, `Ctrl+Tab`, `Alt+Left/Right`, `Esc`).
    /// Returns `true` when `Esc` closed the open drawer viewport.
    fn process_keyboard_nav(
        &mut self,
        ctx: &egui::Context,
        running_exes_empty: bool,
        pending_cmds: &mut Vec<WorkerCommand>,
    ) -> bool {
        if ctx.wants_keyboard_input() || ctx.memory(|m| m.any_popup_open()) {
            return false;
        }
        let (num1, num2, num3, num4, tab_next, tab_prev, alt_left, alt_right, esc) =
            ctx.input(|i| {
                let ctrl_only = i.modifiers.ctrl && !i.modifiers.alt;
                let alt_only = i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift;
                (
                    ctrl_only && i.key_pressed(egui::Key::Num1),
                    ctrl_only && i.key_pressed(egui::Key::Num2),
                    ctrl_only && i.key_pressed(egui::Key::Num3),
                    ctrl_only && i.key_pressed(egui::Key::Num4),
                    ctrl_only && !i.modifiers.shift && i.key_pressed(egui::Key::Tab),
                    ctrl_only && i.modifiers.shift && i.key_pressed(egui::Key::Tab),
                    alt_only && i.key_pressed(egui::Key::ArrowLeft),
                    alt_only && i.key_pressed(egui::Key::ArrowRight),
                    !i.modifiers.ctrl && !i.modifiers.alt && i.key_pressed(egui::Key::Escape),
                )
            });

        let mut changed = false;
        if num1 {
            self.open_drawer = Some(Drawer::Monitor);
            changed = true;
        } else if num2 {
            self.open_drawer = Some(Drawer::FansPower);
            changed = true;
        } else if num3 {
            self.open_drawer = Some(Drawer::Battery);
            changed = true;
        } else if num4 {
            self.open_drawer = Some(Drawer::Extra);
            changed = true;
        } else if tab_next {
            self.open_drawer = Some(
                self.open_drawer
                    .map(|d| d.next())
                    .unwrap_or(Drawer::Monitor),
            );
            changed = true;
        } else if tab_prev {
            self.open_drawer = Some(self.open_drawer.map(|d| d.prev()).unwrap_or(Drawer::Extra));
            changed = true;
        } else if alt_left || alt_right {
            match self.open_drawer {
                Some(Drawer::FansPower) => {
                    self.fans_power_tab = if alt_right {
                        self.fans_power_tab.next()
                    } else {
                        self.fans_power_tab.prev()
                    };
                    changed = true;
                }
                Some(Drawer::Extra) => {
                    self.extra_tab = if alt_right {
                        self.extra_tab.next()
                    } else {
                        self.extra_tab.prev()
                    };
                    changed = true;
                }
                _ => {}
            }
        } else if esc && self.open_drawer.is_some() {
            self.open_drawer = None;
            ctx.request_repaint();
            return true;
        }

        if changed {
            if self.open_drawer == Some(Drawer::FansPower)
                && self.fans_power_tab == FansPowerSubTab::Profiles
                && running_exes_empty
            {
                pending_cmds.push(WorkerCommand::RefreshRunningProcesses);
            }
            ctx.request_repaint();
        }
        false
    }

    fn rebind_hotkeys(&mut self, ctx: &egui::Context) {
        self.omen_action_code.store(
            omen_action_to_code(&self.cfg.hotkeys.omen_key_action),
            Ordering::SeqCst,
        );
        #[cfg(windows)]
        {
            self.stop_hotkeys();

            if self.cfg.hotkeys.capture_omen_key {
                let tx = self.signal_tx.clone();
                let c = ctx.clone();
                let vis = Arc::clone(&self.window_visible);
                let hwnd = Arc::clone(&self.hwnd_atomic);
                let action_code = Arc::clone(&self.omen_action_code);
                self.omen_listener = Some(spawn_omen_key_listener(move || {
                    if action_code.load(Ordering::Relaxed) == 0 {
                        let prev = vis.fetch_xor(true, Ordering::SeqCst);
                        let next = !prev;
                        win32_set_visible(hwnd.load(Ordering::SeqCst), next);
                        let _ = tx.send(AppSignal::SyncVisibility(next));
                    } else {
                        let _ = tx.send(AppSignal::OmenKeyNonToggle);
                    }
                    c.request_repaint();
                }));
            }

            if !self.cfg.hotkeys.global_toggle_shortcut.trim().is_empty() {
                let combo = self.cfg.hotkeys.global_toggle_shortcut.trim().to_string();
                let tx = self.signal_tx.clone();
                let c = ctx.clone();
                let vis = Arc::clone(&self.window_visible);
                let hwnd = Arc::clone(&self.hwnd_atomic);
                match spawn_global_hotkey_listener(&combo, move || {
                    let prev = vis.fetch_xor(true, Ordering::SeqCst);
                    let next = !prev;
                    win32_set_visible(hwnd.load(Ordering::SeqCst), next);
                    let _ = tx.send(AppSignal::SyncVisibility(next));
                    c.request_repaint();
                }) {
                    Ok(handle) => {
                        self.hotkey_listener = Some(handle);
                        self.hotkey_bind_status = ui::hotkeys::HotkeyBindStatus::Bound(combo);
                    }
                    Err(err) => {
                        tracing::warn!("Global hotkey bind failed: {err}");
                        match &err {
                            HotkeyBindError::Conflict { shortcut, .. } => {
                                self.hotkey_bind_status =
                                    ui::hotkeys::HotkeyBindStatus::Conflict(shortcut.clone());
                                if let Ok(mut st) = self.state.lock() {
                                    st.set_toast(
                                        self.i18n
                                            .t("hotkeys.conflict_toast")
                                            .replace("{shortcut}", shortcut),
                                        true,
                                    );
                                }
                            }
                            HotkeyBindError::InvalidSyntax(msg) => {
                                self.hotkey_bind_status =
                                    ui::hotkeys::HotkeyBindStatus::Invalid(msg.clone());
                            }
                            HotkeyBindError::OsError { message, .. } => {
                                self.hotkey_bind_status =
                                    ui::hotkeys::HotkeyBindStatus::Error(message.clone());
                            }
                        }
                    }
                }
            } else {
                self.hotkey_bind_status = ui::hotkeys::HotkeyBindStatus::Disabled;
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    fn sync_window_visibility(&mut self, ctx: &egui::Context, visible: bool) {
        self.window_visible.store(visible, Ordering::SeqCst);
        let hwnd = self.hwnd_atomic.load(Ordering::SeqCst);
        if visible && self.cfg.general.dock_bottom_right {
            win32_dock_bottom_right(hwnd);
        }
        win32_set_visible(hwnd, visible);
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.last_shown_at = Instant::now();
            self.prev_focused = true;
            if self.cfg.general.dock_bottom_right {
                self.pending_dock_frames = 2;
                ctx.request_repaint();
            }
        } else {
            self.open_drawer = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.flush_config_if_dirty();
        }
        if let Some(tray) = &mut self.tray {
            tray.update_labels(&self.i18n, visible);
        }
    }

    fn cycle_performance_mode(&mut self) {
        if self.state.lock().ok().and_then(|s| s.safe_mode).is_some() {
            self.send_cmd(WorkerCommand::SetPerformanceMode(
                self.cfg.thermal_fan.performance_mode,
            ));
            return;
        }
        let next = match self.cfg.thermal_fan.performance_mode {
            PerformanceMode::Eco => PerformanceMode::Quiet,
            PerformanceMode::Quiet => PerformanceMode::Balanced,
            PerformanceMode::Balanced => PerformanceMode::Performance,
            PerformanceMode::Performance => PerformanceMode::Eco,
        };
        self.cfg.thermal_fan.performance_mode = next;
        self.save_config();
        self.send_cmd(WorkerCommand::SetPerformanceMode(next));
        show_mode_change_osd(
            &self.i18n.t("osd.perf_mode_title"),
            &perf_mode_label(&self.i18n, next),
            next,
        );
    }

    fn launch_custom_omen_command(&self) {
        let cmd = self.cfg.hotkeys.custom_command.trim().to_string();
        let args = self.cfg.hotkeys.custom_args.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.send_cmd(WorkerCommand::LaunchCustomCommand { command: cmd, args });
    }

    fn begin_quit(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.open_drawer = None;
        self.force_quit.store(true, Ordering::SeqCst);
        self.clean_quit_started.store(true, Ordering::SeqCst);
        self.save_config();
        self.stop_hotkeys();
        #[cfg(windows)]
        if let Some(pwr) = self.power_listener.take() {
            pwr.stop();
        }
        if let Some(w) = self.worker.take() {
            w.shutdown();
        }
        if let Ok(mut cg) = self.crash_guard.lock() {
            cg.mark_clean_exit(current_unix_secs());
            let _ = cg.save_to_path(&self.crash_guard_path);
        }
        #[cfg(windows)]
        if let Some(mut guard) = self.single_instance.take() {
            guard.release();
        }
        if let Some(h) = &self.log_handle {
            h.flush();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn process_signals(&mut self, ctx: &egui::Context) {
        while let Ok(sig) = self.signal_rx.try_recv() {
            match sig {
                AppSignal::SyncVisibility(visible) => {
                    self.sync_window_visibility(ctx, visible);
                }
                AppSignal::TrayPerfChanged(mode) => {
                    if self.state.lock().ok().and_then(|s| s.safe_mode).is_none() {
                        self.cfg.thermal_fan.performance_mode = mode;
                        self.save_config();
                        show_mode_change_osd(
                            &self.i18n.t("osd.perf_mode_title"),
                            &perf_mode_label(&self.i18n, mode),
                            mode,
                        );
                    } else if let Some(tray) = &mut self.tray {
                        tray.invalidate_checked_cache();
                    }
                }
                AppSignal::TrayFanChanged(mode) => {
                    if self.state.lock().ok().and_then(|s| s.safe_mode).is_none() {
                        self.cfg.thermal_fan.fan_mode = mode;
                        self.save_config();
                        show_mode_change_osd(
                            &self.i18n.t("osd.fan_mode_title"),
                            &fan_mode_label(&self.i18n, mode),
                            self.cfg.thermal_fan.performance_mode,
                        );
                    } else if let Some(tray) = &mut self.tray {
                        tray.invalidate_checked_cache();
                    }
                }
                AppSignal::TrayBatteryPreset {
                    enabled,
                    stop_pct,
                    start_pct,
                } => {
                    if self.state.lock().ok().and_then(|s| s.safe_mode).is_none() {
                        self.cfg.battery.enabled = enabled;
                        self.cfg.battery.stop_threshold_pct = stop_pct;
                        self.cfg.battery.start_threshold_pct = start_pct;
                        self.cfg.battery.clear_travel_charge();
                        self.cfg.battery.mode = if enabled {
                            omvix_core::BatteryChargeMode::Inhibit
                        } else {
                            omvix_core::BatteryChargeMode::Auto
                        };
                        self.cfg.battery.active_profile_id = Some(
                            match stop_pct {
                                60 => "desktop",
                                80 => "balanced",
                                90 => "travel",
                                _ => "full",
                            }
                            .to_string(),
                        );
                        self.save_config();
                        self.send_cmd(WorkerCommand::UpdateConfig(self.cfg.clone()));
                        self.send_cmd(WorkerCommand::ApplyBatteryNow);
                        let sub = if enabled {
                            format!("{stop_pct}%")
                        } else {
                            self.i18n.t("battery.preset_full")
                        };
                        show_mode_change_osd(
                            &self.i18n.t("osd.battery_profile_title"),
                            &sub,
                            self.cfg.thermal_fan.performance_mode,
                        );
                    } else if let Some(tray) = &mut self.tray {
                        tray.invalidate_checked_cache();
                    }
                }
                AppSignal::OmenKeyNonToggle => match self.cfg.hotkeys.omen_key_action {
                    OmenKeyAction::ToggleWindow => {}
                    OmenKeyAction::CyclePerformanceMode => self.cycle_performance_mode(),
                    OmenKeyAction::LaunchCustomCommand => self.launch_custom_omen_command(),
                },
                AppSignal::QuitRequested => self.begin_quit(ctx),
            }
        }
    }

    fn restart_as_admin(&mut self, ctx: &egui::Context) {
        self.flush_config_if_dirty();
        #[cfg(windows)]
        {
            // Release global hotkeys & single-instance mutex BEFORE spawning elevated process.
            self.stop_hotkeys();
            if let Some(guard) = &mut self.single_instance {
                guard.release();
            }

            let in_safe_mode = self.state.lock().ok().and_then(|st| st.safe_mode).is_some();
            if try_shell_elevate_self(in_safe_mode).is_ok() {
                self.begin_quit(ctx);
                return;
            }

            // User cancelled UAC or launch failed -> restore mutex and hotkeys.
            self.single_instance = SingleInstanceGuard::acquire().ok();
            self.rebind_hotkeys(ctx);
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    fn relaunch_updated_exe(&mut self, ctx: &egui::Context, exe_path: &std::path::Path) {
        self.flush_config_if_dirty();
        if let Ok(mut cg) = self.crash_guard.lock() {
            cg.mark_clean_exit(current_unix_secs());
            let _ = cg.save_to_path(&self.crash_guard_path);
        }
        self.stop_hotkeys();
        #[cfg(windows)]
        if let Some(guard) = &mut self.single_instance {
            guard.release();
        }
        match std::process::Command::new(exe_path).spawn() {
            Ok(_) => {
                self.begin_quit(ctx);
            }
            Err(err) => {
                tracing::error!(path = %exe_path.display(), "Failed to relaunch updated binary: {err}");
                if let Ok(mut st) = self.state.lock() {
                    st.set_toast(format!("Failed to relaunch updated binary: {err}"), true);
                }
                #[cfg(windows)]
                {
                    self.single_instance = SingleInstanceGuard::acquire().ok();
                }
                self.rebind_hotkeys(ctx);
            }
        }
    }
}

/// Lightweight proxy sender so tray background actions execute immediately even while window is hidden.
fn worker_command_proxy(worker: &BackgroundWorker) -> Sender<WorkerCommand> {
    worker.sender_clone()
}

impl eframe::App for OmvixApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_signals(ctx);

        if self.pending_dock_frames > 0 {
            self.pending_dock_frames -= 1;
            if self.cfg.general.dock_bottom_right {
                win32_dock_bottom_right(self.hwnd_atomic.load(Ordering::SeqCst));
            }
            if self.pending_dock_frames > 0 {
                ctx.request_repaint();
            }
        } else if self.cfg.general.dock_bottom_right
            && self.window_visible.load(Ordering::Relaxed)
            && !ctx.is_using_pointer()
        {
            win32_dock_bottom_right(self.hwnd_atomic.load(Ordering::SeqCst));
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            let forced = self.force_quit.load(Ordering::SeqCst);
            if !forced && self.cfg.general.minimize_to_tray_on_close && !self.quitting {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.sync_window_visibility(ctx, false);
                tracing::info!("{}", self.i18n.t("app.close_to_tray_notice"));
            } else {
                self.begin_quit(ctx);
            }
        }

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.cfg.general.hide_on_focus_loss
            && !self.show_wizard
            && self.open_drawer.is_none()
            && self.window_visible.load(Ordering::Relaxed)
            && !self.quitting
            && self.pending_dock_frames == 0
            && self.last_shown_at.elapsed() > Duration::from_millis(450)
            && self.prev_focused
            && !focused
            && !ctx.is_using_pointer()
        {
            self.sync_window_visibility(ctx, false);
        }
        self.prev_focused = focused;

        let snapshot_state = self.state.lock().map(|s| s.clone()).unwrap_or_default();

        if snapshot_state.pending_update_relaunch.is_some() {
            let relaunch_path = self
                .state
                .lock()
                .ok()
                .and_then(|mut s| s.pending_update_relaunch.take());
            if let Some(path) = relaunch_path {
                self.relaunch_updated_exe(ctx, &path);
                return;
            }
        }

        let mut pending_cmds: Vec<WorkerCommand> = Vec::new();
        let mut should_save = false;
        if snapshot_state.synced_config_rev != self.last_seen_config_sync_rev {
            self.last_seen_config_sync_rev = snapshot_state.synced_config_rev;
            self.last_seen_auto_revision = snapshot_state.auto_applied_revision;
            if let Some(ref synced) = snapshot_state.synced_config {
                self.cfg = synced.clone();
                self.config_dirty_since = None;
                should_save = true;
            }
        } else if snapshot_state.auto_applied_revision != self.last_seen_auto_revision {
            self.last_seen_auto_revision = snapshot_state.auto_applied_revision;
            if snapshot_state.active_app_profile.is_none() {
                self.cfg.thermal_fan.performance_mode = snapshot_state.effective_performance_mode;
                self.cfg.thermal_fan.fan_mode = snapshot_state.effective_fan_mode;
                should_save = true;
            }
        }

        if let Some(tray) = &mut self.tray {
            let bat_mode = snapshot_state
                .battery_status
                .as_ref()
                .map(|b| b.current_mode)
                .unwrap_or(omvix_core::BatteryChargeMode::Auto);
            let (tray_perf, tray_fan) = if snapshot_state.active_app_profile.is_some() {
                (
                    snapshot_state.effective_performance_mode,
                    snapshot_state.effective_fan_mode,
                )
            } else {
                (
                    self.cfg.thermal_fan.performance_mode,
                    self.cfg.thermal_fan.fan_mode,
                )
            };
            tray.update_visual_state(
                tray_perf,
                tray_fan,
                self.cfg.battery.enabled,
                self.cfg.battery.stop_threshold_pct,
                bat_mode,
                &self.i18n,
            );
        }

        if !self.quitting && self.worker.as_ref().is_some_and(|w| !w.is_alive()) {
            self.worker_dead = true;
            if !self.worker_dead_recovery_attempted {
                self.worker_dead_recovery_attempted = true;
                tracing::error!(
                    "Background worker thread exited unexpectedly; attempting BIOS Auto fan restore"
                );
                #[cfg(windows)]
                thread::spawn(
                    || match omvix_windows::thermal_fan::restore_fan_auto_fresh_wmi() {
                        Ok(()) => {
                            tracing::warn!("Dead-worker recovery restored BIOS Auto fan control");
                        }
                        Err(err) => {
                            tracing::error!("Dead-worker Auto fan recovery failed: {err}");
                        }
                    },
                );
            }
        }

        let current_theme = self.cfg.general.custom_theme.clone();
        let cockpit_frame = egui::Frame::central_panel(&ctx.style())
            .fill(theme::sidebar_fill(&current_theme))
            .inner_margin(Margin::symmetric(12, 8));
        let detail_frame = egui::Frame::central_panel(&ctx.style())
            .fill(theme::rgb(current_theme.canvas_bg))
            .inner_margin(Margin::same(14));

        let mut should_restart_admin = false;
        let mut should_rebind_hotkeys = false;
        let mut should_restore_defaults = false;
        let mut should_hide_to_tray = false;
        let mut should_quit = false;
        let mut toggle_drawer: Option<Drawer> = None;
        let mut open_extra: Option<ExtraSubTab> = None;
        let mut open_profiles = false;

        // Main window ALWAYS renders fixed-size compact cockpit or onboarding wizard (zero resize/flicker)
        egui::CentralPanel::default()
            .frame(cockpit_frame)
            .show(ctx, |ui| {
                let mut push_cmd = |c: WorkerCommand| pending_cmds.push(c);
                let mut mark_save = || should_save = true;
                if self.show_wizard {
                    self.open_drawer = None;
                    let res = ui::wizard::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &mut self.i18n,
                        &mut self.wizard,
                        COMPACT_SIZE[0],
                        &mut push_cmd,
                        &mut mark_save,
                    );
                    if res.language_changed {
                        if let Some(tray) = &mut self.tray {
                            tray.update_labels(
                                &self.i18n,
                                self.window_visible.load(Ordering::Relaxed),
                            );
                        }
                    }
                    if res.restart_admin {
                        should_restart_admin = true;
                    }
                    if res.skipped {
                        self.cfg.general.first_run_completed = true;
                        self.show_wizard = false;
                        should_save = true;
                        pending_cmds.push(WorkerCommand::UpdateConfig(self.cfg.clone()));
                    } else if res.finished {
                        self.cfg.general.first_run_completed = true;
                        self.show_wizard = false;
                        should_save = true;
                        pending_cmds.push(WorkerCommand::UpdateConfig(self.cfg.clone()));
                        if self.cfg.battery.enabled && snapshot_state.safe_mode.is_none() {
                            pending_cmds.push(WorkerCommand::ApplyBatteryNow);
                        }
                    }
                } else {
                    let res = ui::cockpit::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        self.open_drawer,
                        COMPACT_SIZE[0],
                        self.worker_dead,
                        &mut self.config_warnings,
                        &mut push_cmd,
                        &mut mark_save,
                    );
                    toggle_drawer = res.toggle_drawer;
                    open_extra = res.open_extra_subtab;
                    if res.open_profiles {
                        open_profiles = true;
                    }
                    if res.restart_admin {
                        should_restart_admin = true;
                    }
                    if res.hide_to_tray {
                        should_hide_to_tray = true;
                    }
                    if res.quit_app {
                        should_quit = true;
                    }
                }
            });

        if open_profiles {
            self.fans_power_tab = FansPowerSubTab::Profiles;
            self.open_drawer = Some(Drawer::FansPower);
            if snapshot_state.running_exes_snapshot.is_empty() {
                pending_cmds.push(WorkerCommand::RefreshRunningProcesses);
            }
        } else if let Some(sub) = open_extra {
            self.extra_tab = sub;
            self.open_drawer = Some(Drawer::Extra);
        } else if let Some(d) = toggle_drawer {
            self.open_drawer = if self.open_drawer == Some(d) {
                None
            } else {
                Some(d)
            };
        }

        self.process_keyboard_nav(
            ctx,
            snapshot_state.running_exes_snapshot.is_empty(),
            &mut pending_cmds,
        );

        // Secondary flicker-free detail viewport opens to the LEFT of the docked cockpit
        if let Some(drawer) = self.open_drawer {
            if self.window_visible.load(Ordering::Relaxed)
                && !self.quitting
                && !should_hide_to_tray
                && !should_quit
            {
                let drawer_title = match drawer {
                    Drawer::Monitor => self.i18n.t("monitor.title"),
                    Drawer::FansPower => self.i18n.t("drawer.fans_power"),
                    Drawer::Battery => self.i18n.t("nav.battery"),
                    Drawer::Extra => self.i18n.t("drawer.extra"),
                };
                let mut builder = egui::ViewportBuilder::default()
                    .with_title(format!("Omvix Hub — {drawer_title}"))
                    .with_inner_size(DRAWER_SIZE)
                    .with_resizable(false)
                    .with_maximize_button(false)
                    .with_minimize_button(false)
                    .with_taskbar(false);
                if let Some(pos) = win32_drawer_anchor_pos(
                    self.hwnd_atomic.load(Ordering::SeqCst),
                    DRAWER_SIZE[0],
                    DRAWER_SIZE[1],
                    ctx.pixels_per_point(),
                ) {
                    builder = builder.with_position(pos);
                }

                let mut close_drawer = false;
                let mut theme_applied = false;
                let drawer_id = egui::ViewportId::from_hash_of("omvix_drawer_viewport");
                ctx.show_viewport_immediate(drawer_id, builder, |drawer_ctx, _class| {
                    if drawer_ctx.input(|i| i.viewport().close_requested()) {
                        close_drawer = true;
                        return;
                    }
                    if self.process_keyboard_nav(
                        drawer_ctx,
                        snapshot_state.running_exes_snapshot.is_empty(),
                        &mut pending_cmds,
                    ) {
                        close_drawer = true;
                        return;
                    }
                    let expected_font_px =
                        theme::expected_body_font_size(self.cfg.general.font_scale_pct);
                    let drawer_font_px = drawer_ctx
                        .style()
                        .text_styles
                        .get(&egui::TextStyle::Body)
                        .map(|f| f.size)
                        .unwrap_or(0.0);
                    if drawer_ctx.style().visuals.panel_fill != theme::rgb(current_theme.canvas_bg)
                        || (drawer_font_px - expected_font_px).abs() > 0.1
                    {
                        theme::apply_theme(
                            drawer_ctx,
                            &self.cfg.general.custom_theme,
                            self.cfg.general.font_scale_pct,
                        );
                    }

                    egui::CentralPanel::default()
                        .frame(detail_frame)
                        .show(drawer_ctx, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                match drawer {
                                    Drawer::Monitor => {
                                        ui.heading(self.i18n.t("monitor.title"));
                                    }
                                    Drawer::FansPower => {
                                        for (sub, key) in [
                                            (
                                                FansPowerSubTab::PerformanceFans,
                                                "drawer.sub_perf_fans",
                                            ),
                                            (FansPowerSubTab::Profiles, "drawer.sub_profiles"),
                                            (FansPowerSubTab::Tuning, "drawer.sub_cpu_power"),
                                        ] {
                                            if theme::tile_button(
                                                ui,
                                                &self.i18n.t(key),
                                                self.fans_power_tab == sub,
                                                152.0,
                                                28.0,
                                                &current_theme,
                                            )
                                            .clicked()
                                            {
                                                self.fans_power_tab = sub;
                                                if sub == FansPowerSubTab::Profiles
                                                    && snapshot_state
                                                        .running_exes_snapshot
                                                        .is_empty()
                                                {
                                                    pending_cmds.push(
                                                        WorkerCommand::RefreshRunningProcesses,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Drawer::Battery => {
                                        ui.heading(self.i18n.t("nav.battery"));
                                    }
                                    Drawer::Extra => {
                                        for (sub, key) in [
                                            (
                                                ExtraSubTab::LightingDisplay,
                                                "drawer.sub_lighting_display",
                                            ),
                                            (ExtraSubTab::Hotkeys, "drawer.sub_hotkeys"),
                                            (ExtraSubTab::Diagnostics, "drawer.sub_diagnostics"),
                                            (ExtraSubTab::Settings, "drawer.sub_settings"),
                                        ] {
                                            if theme::tile_button(
                                                ui,
                                                &self.i18n.t(key),
                                                self.extra_tab == sub,
                                                114.0,
                                                28.0,
                                                &current_theme,
                                            )
                                            .clicked()
                                            {
                                                self.extra_tab = sub;
                                            }
                                        }
                                    }
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if theme::tile_button(
                                            ui,
                                            &self.i18n.t("drawer.collapse"),
                                            false,
                                            88.0,
                                            28.0,
                                            &current_theme,
                                        )
                                        .clicked()
                                        {
                                            close_drawer = true;
                                        }
                                    },
                                );
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut push_cmd = |c: WorkerCommand| pending_cmds.push(c);
                                let mut mark_save = || should_save = true;

                                match drawer {
                                    Drawer::Monitor => ui::monitor::render(
                                        ui,
                                        &mut self.cfg,
                                        &snapshot_state,
                                        &self.i18n,
                                        &mut push_cmd,
                                        &mut mark_save,
                                    ),
                                    Drawer::FansPower => match self.fans_power_tab {
                                        FansPowerSubTab::PerformanceFans => {
                                            ui::performance::render(
                                                ui,
                                                &mut self.cfg,
                                                &snapshot_state,
                                                &self.i18n,
                                                &mut self.performance_draft,
                                                &mut push_cmd,
                                                &mut mark_save,
                                            )
                                        }
                                        FansPowerSubTab::Profiles => ui::profiles::render(
                                            ui,
                                            &mut self.cfg,
                                            &snapshot_state,
                                            &self.i18n,
                                            &mut self.profiles_draft,
                                            &mut push_cmd,
                                            &mut mark_save,
                                        ),
                                        FansPowerSubTab::Tuning => ui::tuning::render(
                                            ui,
                                            &mut self.cfg,
                                            &snapshot_state,
                                            &self.i18n,
                                            &mut push_cmd,
                                            &mut mark_save,
                                        ),
                                    },
                                    Drawer::Battery => ui::battery::render(
                                        ui,
                                        &mut self.cfg,
                                        &snapshot_state,
                                        &self.i18n,
                                        &mut self.battery_draft,
                                        &mut push_cmd,
                                        &mut mark_save,
                                    ),
                                    Drawer::Extra => match self.extra_tab {
                                        ExtraSubTab::LightingDisplay => {
                                            ui::lighting_display::render(
                                                ui,
                                                &mut self.cfg,
                                                &snapshot_state,
                                                &self.i18n,
                                                &mut self.lighting_draft,
                                                &mut push_cmd,
                                                &mut mark_save,
                                            )
                                        }
                                        ExtraSubTab::Hotkeys => ui::hotkeys::render(
                                            ui,
                                            &mut self.cfg,
                                            &self.i18n,
                                            &self.hotkey_bind_status,
                                            &self.config_warnings,
                                            &mut push_cmd,
                                            &mut mark_save,
                                            &mut || should_rebind_hotkeys = true,
                                        ),
                                        ExtraSubTab::Diagnostics => ui::diagnostics::render(
                                            ui,
                                            &snapshot_state,
                                            &self.i18n,
                                            &current_theme,
                                            &mut push_cmd,
                                        ),
                                        ExtraSubTab::Settings => {
                                            let res = ui::settings::render(
                                                ui,
                                                &mut self.cfg,
                                                &snapshot_state,
                                                &mut self.i18n,
                                                &mut self.settings_draft,
                                                &mut push_cmd,
                                                &mut mark_save,
                                            );
                                            if res.language_changed {
                                                if let Some(tray) = &mut self.tray {
                                                    tray.update_labels(
                                                        &self.i18n,
                                                        self.window_visible.load(Ordering::Relaxed),
                                                    );
                                                }
                                            }
                                            if res.theme_changed {
                                                theme_applied = true;
                                                theme::apply_theme(
                                                    drawer_ctx,
                                                    &self.cfg.general.custom_theme,
                                                    self.cfg.general.font_scale_pct,
                                                );
                                            }
                                            if res.dock_changed
                                                && self.cfg.general.dock_bottom_right
                                            {
                                                self.pending_dock_frames = 2;
                                                ctx.request_repaint();
                                            }
                                            if res.open_wizard {
                                                self.open_drawer = None;
                                                self.wizard.step = 0;
                                                self.show_wizard = true;
                                            }
                                            if res.restore_defaults {
                                                should_restore_defaults = true;
                                            }
                                        }
                                    },
                                }
                            });
                        });
                });

                if close_drawer {
                    self.open_drawer = None;
                }
                if theme_applied {
                    theme::apply_theme(
                        ctx,
                        &self.cfg.general.custom_theme,
                        self.cfg.general.font_scale_pct,
                    );
                }
            }
        }

        if should_restore_defaults {
            let keep_lang = self.cfg.general.language.clone();
            self.cfg = AppConfig::default();
            self.cfg.general.language = keep_lang;
            self.cfg.general.start_minimized = false;
            self.cfg.general.first_run_completed = true;
            self.config_warnings.clear();
            self.performance_draft = ui::performance::PerformanceDraftState::default();
            self.profiles_draft = ui::profiles::ProfilesDraftState::default();
            self.battery_draft = ui::battery::BatteryDraftState::default();
            self.lighting_draft = ui::lighting_display::LightingDisplayDraftState::default();
            self.settings_draft = ui::settings::SettingsThemeDraft::default();
            self.last_seen_config_sync_rev = snapshot_state.synced_config_rev;
            self.last_seen_auto_revision = snapshot_state.auto_applied_revision;
            theme::apply_theme(
                ctx,
                &self.cfg.general.custom_theme,
                self.cfg.general.font_scale_pct,
            );
            if let Some(tray) = &mut self.tray {
                tray.update_labels(&self.i18n, self.window_visible.load(Ordering::Relaxed));
            }
            should_save = true;
            should_rebind_hotkeys = true;
            if self.cfg.general.dock_bottom_right {
                self.pending_dock_frames = 2;
            }
            pending_cmds.push(WorkerCommand::UpdateConfig(self.cfg.clone()));
            pending_cmds.push(WorkerCommand::RestoreAllDefaults);
        }

        if should_save {
            self.omen_action_code.store(
                omen_action_to_code(&self.cfg.hotkeys.omen_key_action),
                Ordering::SeqCst,
            );
            if should_restore_defaults || !self.window_visible.load(Ordering::Relaxed) {
                self.save_config();
            } else {
                self.config_dirty_since = Some(Instant::now());
                ctx.request_repaint_after(Duration::from_millis(450));
            }
        }
        for cmd in pending_cmds {
            self.send_cmd(cmd);
        }
        if should_rebind_hotkeys {
            self.rebind_hotkeys(ctx);
        }
        if should_restart_admin {
            self.restart_as_admin(ctx);
        }
        if should_hide_to_tray {
            self.sync_window_visibility(ctx, false);
        }
        if should_quit {
            self.begin_quit(ctx);
        } else if self
            .config_dirty_since
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(400))
        {
            self.save_config();
        }
    }
}
