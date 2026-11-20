#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod theme;
mod tray;
mod ui;
mod worker;

use crate::tray::{TrayAction, TrayController, build_tray_icon_rgba};
use crate::ui::Tab;
use crate::worker::{BackgroundWorker, SharedState, WorkerCommand};
use egui::{Color32, RichText};
use omvix_core::config::OmenKeyAction;
use omvix_core::{AppConfig, I18n, LogHandle, PerformanceMode, init_logging};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use omvix_windows::hotkeys::{ListenerHandle, spawn_global_hotkey_listener, spawn_omen_key_listener};

#[derive(Debug, Clone)]
enum AppSignal {
    Tray(TrayAction),
    OmenKeyPressed,
    GlobalTogglePressed,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let smoke_test = args.iter().any(|a| a == "--smoke-test");

    let cfg_path = AppConfig::default_config_path();
    let cfg = AppConfig::load_from_path(&cfg_path).unwrap_or_default();
    let log_handle = init_logging(&cfg.logging, smoke_test).ok();
    let i18n = I18n::new(&cfg.general.language);

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

    tracing::info!(lang = %i18n.active_language(), "Launching Omvix Hub desktop GUI");

    let (rgba, icon_w, icon_h) = build_tray_icon_rgba();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Omvix Hub")
        .with_inner_size([1120.0, 740.0])
        .with_min_inner_size([900.0, 600.0])
        .with_visible(!cfg.general.start_minimized)
        .with_icon(Arc::new(egui::IconData {
            rgba,
            width: icon_w,
            height: icon_h,
        }));

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let tray_ctrl = match TrayController::new(&i18n) {
        Ok(t) => Some(t),
        Err(err) => {
            tracing::warn!("Could not initialize system tray icon: {err}");
            None
        }
    };

    if let Err(err) = eframe::run_native(
        "Omvix Hub",
        options,
        Box::new(move |cc| {
            theme::configure_fonts(&cc.egui_ctx);
            theme::apply_theme(&cc.egui_ctx, cfg.general.theme != "light");
            Ok(Box::new(OmvixApp::new(
                cc.egui_ctx.clone(),
                cfg,
                cfg_path,
                i18n,
                tray_ctrl,
                log_handle,
            )))
        }),
    ) {
        tracing::error!("eframe exited with error: {err}");
    }
}

struct OmvixApp {
    cfg: AppConfig,
    cfg_path: PathBuf,
    i18n: I18n,
    active_tab: Tab,
    window_visible: bool,
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
    log_handle: Option<LogHandle>,
}

impl OmvixApp {
    fn new(
        ctx: egui::Context,
        cfg: AppConfig,
        cfg_path: PathBuf,
        i18n: I18n,
        tray: Option<TrayController>,
        log_handle: Option<LogHandle>,
    ) -> Self {
        let worker = BackgroundWorker::spawn(cfg.clone(), Some(ctx.clone()));
        let state = worker.state();
        let (signal_tx, signal_rx) = mpsc::channel::<AppSignal>();

        // Poll tray events every 80ms even while the main window is hidden in tray.
        let tray_tx = signal_tx.clone();
        let wake_ctx = ctx.clone();
        thread::spawn(move || loop {
            let mut woke = false;
            for action in TrayController::poll_actions() {
                let _ = tray_tx.send(AppSignal::Tray(action));
                woke = true;
            }
            if woke {
                wake_ctx.request_repaint();
            }
            thread::sleep(Duration::from_millis(80));
        });

        let window_visible = !cfg.general.start_minimized;
        let mut app = Self {
            cfg,
            cfg_path,
            i18n,
            active_tab: Tab::Dashboard,
            window_visible,
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
            log_handle,
        };
        app.rebind_hotkeys(&ctx);
        app
    }

    fn save_config(&self) {
        if let Err(err) = self.cfg.save_to_path(&self.cfg_path) {
            tracing::warn!("Failed to save config: {err}");
        }
    }

    fn send_cmd(&self, cmd: WorkerCommand) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    fn rebind_hotkeys(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            if let Some(old) = self.omen_listener.take() {
                old.stop();
            }
            if let Some(old) = self.hotkey_listener.take() {
                old.stop();
            }

            if self.cfg.hotkeys.capture_omen_key {
                let tx = self.signal_tx.clone();
                let c = ctx.clone();
                self.omen_listener = Some(spawn_omen_key_listener(move || {
                    let _ = tx.send(AppSignal::OmenKeyPressed);
                    c.request_repaint();
                }));
            }

            if !self.cfg.hotkeys.global_toggle_shortcut.trim().is_empty() {
                let tx = self.signal_tx.clone();
                let c = ctx.clone();
                match spawn_global_hotkey_listener(&self.cfg.hotkeys.global_toggle_shortcut, move || {
                    let _ = tx.send(AppSignal::GlobalTogglePressed);
                    c.request_repaint();
                }) {
                    Ok(handle) => self.hotkey_listener = Some(handle),
                    Err(err) => tracing::warn!("Global hotkey bind failed: {err}"),
                }
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    fn toggle_window(&mut self, ctx: &egui::Context) {
        self.window_visible = !self.window_visible;
        if self.window_visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if let Some(tray) = &self.tray {
            tray.update_labels(&self.i18n, self.window_visible);
        }
    }

    fn cycle_performance_mode(&mut self) {
        let next = match self.cfg.thermal_fan.performance_mode {
            PerformanceMode::Eco => PerformanceMode::Quiet,
            PerformanceMode::Quiet => PerformanceMode::Balanced,
            PerformanceMode::Balanced => PerformanceMode::Performance,
            PerformanceMode::Performance => PerformanceMode::Eco,
        };
        self.cfg.thermal_fan.performance_mode = next;
        self.save_config();
        self.send_cmd(WorkerCommand::SetPerformanceMode(next));
    }

    fn launch_custom_omen_command(&self) {
        let cmd = self.cfg.hotkeys.custom_command.trim();
        if cmd.is_empty() {
            return;
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("cmd").args(["/C", cmd]).spawn();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("sh").args(["-c", cmd]).spawn();
        }
    }

    fn begin_quit(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.save_config();

        #[cfg(windows)]
        {
            if let Some(old) = self.omen_listener.take() {
                old.stop();
            }
            if let Some(old) = self.hotkey_listener.take() {
                old.stop();
            }
        }
        if let Some(w) = self.worker.take() {
            w.shutdown();
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
                AppSignal::Tray(action) => match action {
                    TrayAction::ToggleWindow => self.toggle_window(ctx),
                    TrayAction::SetPerformance(mode) => {
                        self.cfg.thermal_fan.performance_mode = mode;
                        self.save_config();
                        self.send_cmd(WorkerCommand::SetPerformanceMode(mode));
                    }
                    TrayAction::SetFan(mode) => {
                        self.cfg.thermal_fan.fan_mode = mode;
                        self.save_config();
                        self.send_cmd(WorkerCommand::SetFanMode {
                            mode,
                            manual_speed_pct: Some(self.cfg.thermal_fan.manual_speed_pct),
                        });
                    }
                    TrayAction::ToggleBatteryLimit => {
                        self.cfg.battery.enabled = !self.cfg.battery.enabled;
                        self.save_config();
                        self.send_cmd(WorkerCommand::UpdateConfig(self.cfg.clone()));
                        self.send_cmd(WorkerCommand::ApplyBatteryNow);
                    }
                    TrayAction::Quit => self.begin_quit(ctx),
                },
                AppSignal::GlobalTogglePressed => self.toggle_window(ctx),
                AppSignal::OmenKeyPressed => match self.cfg.hotkeys.omen_key_action {
                    OmenKeyAction::ToggleWindow => self.toggle_window(ctx),
                    OmenKeyAction::CyclePerformanceMode => self.cycle_performance_mode(),
                    OmenKeyAction::LaunchCustomCommand => self.launch_custom_omen_command(),
                },
            }
        }
    }

    fn restart_as_admin(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::Shell::ShellExecuteW;
            use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
            use windows::core::{HSTRING, PCWSTR};

            if let Ok(exe) = std::env::current_exe() {
                let verb = HSTRING::from("runas");
                let file = HSTRING::from(exe.to_string_lossy().as_ref());
                let res = unsafe {
                    ShellExecuteW(
                        HWND::default(),
                        PCWSTR(verb.as_ptr()),
                        PCWSTR(file.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    )
                };
                if res.0 as isize > 32 {
                    self.begin_quit(ctx);
                } else {
                    tracing::warn!("ShellExecuteW(runas) declined or failed: {:?}", res.0);
                }
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }
}

impl eframe::App for OmvixApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_signals(ctx);

        if ctx.input(|i| i.viewport().close_requested()) {
            if self.cfg.general.minimize_to_tray_on_close && !self.quitting {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.window_visible = false;
                if let Some(tray) = &self.tray {
                    tray.update_labels(&self.i18n, false);
                }
                tracing::info!("{}", self.i18n.t("app.close_to_tray_notice"));
            } else {
                self.begin_quit(ctx);
            }
        }

        let dark = self.cfg.general.theme != "light";
        let snapshot_state = self
            .state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();

        // Sidebar navigation
        egui::SidePanel::left("omvix_sidebar")
            .resizable(false)
            .exact_width(210.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(self.i18n.t("app.title"))
                        .size(21.0)
                        .strong()
                        .color(theme::ACCENT_CYAN),
                );
                ui.weak(self.i18n.t("app.subtitle"));
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);

                for &tab in Tab::all() {
                    let selected = self.active_tab == tab;
                    let label = self.i18n.t(tab.i18n_key());
                    let btn = egui::Button::new(RichText::new(label).size(14.5)).selected(selected);
                    if ui.add_sized([190.0, 32.0], btn).clicked() {
                        self.active_tab = tab;
                    }
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    if let Some((msg, is_err, ts)) = &snapshot_state.last_toast {
                        if ts.elapsed() < Duration::from_secs(6) {
                            let color = if *is_err {
                                Color32::from_rgb(239, 68, 68)
                            } else {
                                Color32::from_rgb(34, 197, 94)
                            };
                            ui.colored_label(color, msg);
                        }
                    }
                    ui.weak(format!("v{}", env!("CARGO_PKG_VERSION")));
                });
            });

        // Main content area
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut pending_cmds: Vec<WorkerCommand> = Vec::new();
                let mut should_save = false;
                let mut should_restart_admin = false;
                let mut should_rebind_hotkeys = false;

                let mut push_cmd = |c: WorkerCommand| pending_cmds.push(c);
                let mut mark_save = || should_save = true;

                match self.active_tab {
                    Tab::Dashboard => ui::dashboard::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut || should_restart_admin = true,
                    ),
                    Tab::PerformanceFans => ui::performance::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut mark_save,
                    ),
                    Tab::Battery => ui::battery::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut mark_save,
                    ),
                    Tab::LightingDisplay => ui::lighting_display::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut mark_save,
                    ),
                    Tab::Tuning => ui::tuning::render(
                        ui,
                        &mut self.cfg,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut mark_save,
                    ),
                    Tab::Hotkeys => ui::hotkeys::render(
                        ui,
                        &mut self.cfg,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                        &mut mark_save,
                        &mut || should_rebind_hotkeys = true,
                    ),
                    Tab::Diagnostics => ui::diagnostics::render(
                        ui,
                        &snapshot_state,
                        &self.i18n,
                        dark,
                        &mut push_cmd,
                    ),
                    Tab::Settings => {
                        let res = ui::settings::render(
                            ui,
                            &mut self.cfg,
                            &mut self.i18n,
                            dark,
                            &mut push_cmd,
                            &mut mark_save,
                        );
                        if res.language_changed {
                            if let Some(tray) = &self.tray {
                                tray.update_labels(&self.i18n, self.window_visible);
                            }
                        }
                        if res.theme_changed {
                            theme::apply_theme(ctx, self.cfg.general.theme != "light");
                        }
                    }
                }

                if should_save {
                    self.save_config();
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
            });
        });
    }
}
