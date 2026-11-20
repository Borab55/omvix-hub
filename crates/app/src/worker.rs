use omvix_core::auto_profile::AutoProfileEngine;
use omvix_core::battery_care::BatteryCareEngine;
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryLimitBackend, BatteryStatus, ConflictServiceInfo,
    CpuBoostMode, CpuTuningBackend, DiagnosticReport, DiagnosticsBackend, DisplayBackend,
    DisplayModeInfo, FanMode, GpuMode, GpuSwitchBackend, KeyboardBackend, KeyboardType,
    PerformanceMode, Result, SecurityStatus, TelemetryBackend, TelemetrySnapshot,
    ThermalFanBackend,
};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
use omvix_windows::{
    WindowsBatteryBackend, WindowsCpuTuningBackend, WindowsDiagnosticsBackend,
    WindowsDisplayBackend, WindowsGpuSwitchBackend, WindowsKeyboardBackend,
    WindowsTelemetryBackend, WindowsThermalFanBackend, system_info,
};

const MAX_HISTORY_SAMPLES: usize = 60;

#[derive(Debug, Clone)]
pub struct SharedState {
    pub telemetry: TelemetrySnapshot,
    pub cpu_usage_history: VecDeque<(f64, f32)>,
    pub cpu_temp_history: VecDeque<(f64, f32)>,
    pub gpu_temp_history: VecDeque<(f64, f32)>,
    pub gpu_load_history: VecDeque<(f64, f32)>,
    pub battery_status: Option<BatteryStatus>,
    pub display_mode: Option<DisplayModeInfo>,
    pub diagnostics: Option<DiagnosticReport>,
    pub cpu_boost: Option<CpuBoostMode>,
    pub gpu_mode: Option<GpuMode>,
    pub keyboard_type: KeyboardType,
    pub security: SecurityStatus,
    pub conflict_services: Vec<ConflictServiceInfo>,
    pub hp_charge_service_running: bool,
    pub last_toast: Option<(String, bool, Instant)>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            telemetry: TelemetrySnapshot::default(),
            cpu_usage_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            cpu_temp_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            gpu_temp_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            gpu_load_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            battery_status: None,
            display_mode: None,
            diagnostics: None,
            cpu_boost: None,
            gpu_mode: None,
            keyboard_type: KeyboardType::Unknown,
            security: SecurityStatus {
                is_admin: false,
                vbs_status: None,
                hvci_running: false,
                pawn_io_installed: false,
                undervolt_note: String::new(),
            },
            conflict_services: Vec::new(),
            hp_charge_service_running: false,
            last_toast: None,
        }
    }
}

impl SharedState {
    pub fn set_toast(&mut self, msg: impl Into<String>, is_error: bool) {
        self.last_toast = Some((msg.into(), is_error, Instant::now()));
    }
}

#[derive(Debug, Clone)]
pub enum WorkerCommand {
    SetPerformanceMode(PerformanceMode),
    SetFanMode {
        mode: FanMode,
        manual_speed_pct: Option<u8>,
    },
    UpdateConfig(AppConfig),
    ApplyBatteryNow,
    ApplyLighting,
    SetRefreshRate(u32),
    SetGpuMode(GpuMode),
    SetCpuBoost(CpuBoostMode),
    SetPowerLimits {
        pl1: Option<u16>,
        pl2: Option<u16>,
    },
    SetTccLimit(u8),
    SetUndervolt(i16),
    RefreshDiagnostics,
    ExportDiagnostics(PathBuf),
    Shutdown,
}

pub struct BackgroundWorker {
    tx: Sender<WorkerCommand>,
    state: Arc<Mutex<SharedState>>,
    join: Option<JoinHandle<()>>,
}

impl BackgroundWorker {
    pub fn spawn(initial_config: AppConfig, ctx: Option<egui::Context>) -> Self {
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        let state = Arc::new(Mutex::new(SharedState::default()));
        let state_bg = Arc::clone(&state);

        let join = thread::spawn(move || {
            run_worker_loop(initial_config, state_bg, rx, ctx);
        });

        Self {
            tx,
            state,
            join: Some(join),
        }
    }

    pub fn send(&self, cmd: WorkerCommand) {
        let _ = self.tx.send(cmd);
    }

    pub fn state(&self) -> Arc<Mutex<SharedState>> {
        Arc::clone(&self.state)
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(WorkerCommand::Shutdown);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

fn push_sample(buf: &mut VecDeque<(f64, f32)>, x: f64, y: f32) {
    if buf.len() >= MAX_HISTORY_SAMPLES {
        buf.pop_front();
    }
    buf.push_back((x, y));
}

#[cfg(windows)]
fn run_worker_loop(
    mut cfg: AppConfig,
    state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    ctx: Option<egui::Context>,
) {
    let diag_be = WindowsDiagnosticsBackend::new();
    let telemetry_be = WindowsTelemetryBackend::new();
    let thermal_be = WindowsThermalFanBackend::new();
    let battery_be = WindowsBatteryBackend::new();
    let kbd_be = WindowsKeyboardBackend::new();
    let gpu_be = WindowsGpuSwitchBackend::new();
    let display_be = WindowsDisplayBackend::new();
    let tuning_be = WindowsCpuTuningBackend::new();

    let mut battery_engine = BatteryCareEngine::new();
    let mut auto_profile_engine = AutoProfileEngine::new();

    // Initial discovery snapshot
    {
        let sec = system_info::collect_security_status();
        let conflicts = system_info::scan_conflict_services();
        let hp_charge_running = conflicts.iter().any(|s| {
            s.service_name.eq_ignore_ascii_case("hp-charge-limit-rs")
                && s.status.eq_ignore_ascii_case("Running")
        });
        let disp = display_be.get_display_mode().ok();
        let boost = tuning_be.read_cpu_boost_mode().ok();
        let kbd_type = kbd_be.detect_keyboard_type().unwrap_or(KeyboardType::Unknown);
        let bat = battery_be.get_battery_status().ok();
        let gpu_mode = if sec.is_admin {
            gpu_be.get_gpu_mode().ok()
        } else {
            None
        };
        let diag = diag_be.collect_diagnostics().ok();

        if let Ok(mut st) = state.lock() {
            st.security = sec;
            st.conflict_services = conflicts;
            st.hp_charge_service_running = hp_charge_running;
            st.display_mode = disp;
            st.cpu_boost = boost;
            st.keyboard_type = kbd_type;
            st.battery_status = bat;
            st.gpu_mode = gpu_mode;
            st.diagnostics = diag;
        }
        if let Some(c) = &ctx {
            c.request_repaint();
        }
    }

    let started = Instant::now();
    let mut last_telemetry = Instant::now() - Duration::from_secs(10);
    let mut last_battery = Instant::now() - Duration::from_secs(10);
    let mut last_curve_eval = Instant::now();
    let mut last_keep_alive = Instant::now();
    let mut prev_curve_temp: f32 = 45.0;
    let mut prev_curve_speed: u8 = 40;
    let mut last_applied_bat_mode: Option<BatteryChargeMode> = None;

    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WorkerCommand::Shutdown => {
                    tracing::info!("Background worker shutting down");
                    return;
                }
                WorkerCommand::UpdateConfig(new_cfg) => {
                    cfg = new_cfg;
                }
                WorkerCommand::SetPerformanceMode(mode) => {
                    cfg.thermal_fan.performance_mode = mode;
                    let res = thermal_be.set_performance_mode(mode);
                    record_result(&state, res, format!("Performance mode -> {mode:?}"));
                }
                WorkerCommand::SetFanMode {
                    mode,
                    manual_speed_pct,
                } => {
                    cfg.thermal_fan.fan_mode = mode;
                    if let Some(pct) = manual_speed_pct {
                        cfg.thermal_fan.manual_speed_pct = pct;
                    }
                    let res = thermal_be.set_fan_mode(mode, manual_speed_pct);
                    record_result(&state, res, format!("Fan mode -> {mode:?}"));
                }
                WorkerCommand::ApplyBatteryNow => {
                    if let Ok(bat) = battery_be.get_battery_status() {
                        let target = battery_engine.evaluate(
                            &mut cfg.battery,
                            bat.charge_percent,
                            bat.ac_connected,
                        );
                        let res = battery_be.set_charge_mode(target);
                        if res.is_ok() {
                            last_applied_bat_mode = Some(target);
                        }
                        record_result(&state, res, format!("Battery charge mode -> {target:?}"));
                    }
                }
                WorkerCommand::ApplyLighting => {
                    let res = kbd_be.set_backlight(
                        cfg.lighting.enabled,
                        cfg.lighting.brightness_pct,
                        &cfg.lighting.zones,
                    );
                    record_result(&state, res, "Keyboard lighting updated".to_string());
                }
                WorkerCommand::SetRefreshRate(hz) => {
                    let res = display_be.set_refresh_rate(hz);
                    if res.is_ok() {
                        if let Ok(disp) = display_be.get_display_mode() {
                            if let Ok(mut st) = state.lock() {
                                st.display_mode = Some(disp);
                            }
                        }
                    }
                    record_result(&state, res, format!("Refresh rate -> {hz} Hz"));
                }
                WorkerCommand::SetGpuMode(mode) => {
                    let res = gpu_be.set_gpu_mode(mode);
                    if res.is_ok() {
                        if let Ok(mut st) = state.lock() {
                            st.gpu_mode = Some(mode);
                        }
                    }
                    record_result(
                        &state,
                        res,
                        format!("GPU MUX -> {mode:?} (reboot required)"),
                    );
                }
                WorkerCommand::SetCpuBoost(mode) => {
                    let res = tuning_be.set_cpu_boost_mode(mode);
                    if res.is_ok() {
                        if let Ok(mut st) = state.lock() {
                            st.cpu_boost = Some(mode);
                        }
                    }
                    record_result(&state, res, format!("CPU Boost -> {mode:?}"));
                }
                WorkerCommand::SetPowerLimits { pl1, pl2 } => {
                    let res = tuning_be.set_power_limits(pl1, pl2);
                    record_result(&state, res, format!("Power limits PL1={pl1:?} PL2={pl2:?}"));
                }
                WorkerCommand::SetTccLimit(temp_c) => {
                    let res = tuning_be.set_tcc_temp_limit(temp_c);
                    record_result(&state, res, format!("TCC temp limit -> {temp_c}°C"));
                }
                WorkerCommand::SetUndervolt(mv) => {
                    let res = tuning_be.set_core_undervolt_mv(mv);
                    record_result(&state, res, format!("Core undervolt -> {mv} mV"));
                }
                WorkerCommand::RefreshDiagnostics => {
                    let res = diag_be.collect_diagnostics();
                    match res {
                        Ok(report) => {
                            if let Ok(mut st) = state.lock() {
                                st.security = report.security.clone();
                                st.conflict_services = report.conflict_services.clone();
                                st.diagnostics = Some(report);
                                st.set_toast("Diagnostics refreshed", false);
                            }
                        }
                        Err(err) => {
                            if let Ok(mut st) = state.lock() {
                                st.set_toast(err.to_string(), true);
                            }
                        }
                    }
                }
                WorkerCommand::ExportDiagnostics(path) => match diag_be.collect_diagnostics() {
                    Ok(report) => match serde_json::to_string_pretty(&report) {
                        Ok(json) => {
                            let res = fs::write(&path, json).map_err(Into::into);
                            record_result(
                                &state,
                                res,
                                format!("Exported diagnostics to {}", path.display()),
                            );
                        }
                        Err(err) => {
                            if let Ok(mut st) = state.lock() {
                                st.set_toast(err.to_string(), true);
                            }
                        }
                    },
                    Err(err) => {
                        if let Ok(mut st) = state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                },
            }
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        }

        // Periodic telemetry poll (1.5s)
        if last_telemetry.elapsed() >= Duration::from_millis(1500) {
            last_telemetry = Instant::now();
            if let Ok(snap) = telemetry_be.read_telemetry() {
                let t = started.elapsed().as_secs_f64();
                if let Ok(mut st) = state.lock() {
                    if let Some(u) = snap.cpu_usage_pct {
                        push_sample(&mut st.cpu_usage_history, t, u);
                    }
                    if let Some(ct) = snap.cpu_temp_c {
                        push_sample(&mut st.cpu_temp_history, t, ct);
                    }
                    if let Some(gt) = snap.gpu_temp_c {
                        push_sample(&mut st.gpu_temp_history, t, gt);
                    }
                    if let Some(gl) = snap.gpu_usage_pct {
                        push_sample(&mut st.gpu_load_history, t, gl);
                    }
                    st.telemetry = snap.clone();
                }

                // Evaluate AC/DC Auto-Profile on transitions
                if let Some(ac) = snap.ac_connected {
                    if let Some(action) = auto_profile_engine.evaluate(&cfg.auto_profile, ac, false)
                    {
                        tracing::info!(?action, "AC/DC transition triggered auto-profile");
                        let _ = thermal_be.set_performance_mode(action.performance_mode);
                        if let Some(hz) = action.refresh_hz {
                            if display_be.set_refresh_rate(hz).is_ok() {
                                if let Ok(disp) = display_be.get_display_mode() {
                                    if let Ok(mut st) = state.lock() {
                                        st.display_mode = Some(disp);
                                    }
                                }
                            }
                        }
                        if action.dim_keyboard {
                            let _ = kbd_be.set_backlight(false, 0, &cfg.lighting.zones);
                        } else if cfg.lighting.enabled {
                            let _ = kbd_be.set_backlight(
                                true,
                                cfg.lighting.brightness_pct,
                                &cfg.lighting.zones,
                            );
                        }
                    }
                }

                // Evaluate custom fan curve (every 2s)
                if cfg.thermal_fan.fan_mode == FanMode::CustomCurve
                    && last_curve_eval.elapsed() >= Duration::from_secs(2)
                {
                    last_curve_eval = Instant::now();
                    let temp = snap
                        .cpu_temp_c
                        .unwrap_or(0.0)
                        .max(snap.gpu_temp_c.unwrap_or(0.0));
                    if temp > 10.0 {
                        let target_pct = cfg.thermal_fan.custom_curve.evaluate(
                            temp,
                            prev_curve_temp,
                            prev_curve_speed,
                        );
                        if target_pct != prev_curve_speed {
                            let _ = thermal_be.set_fan_mode(FanMode::CustomCurve, Some(target_pct));
                            prev_curve_speed = target_pct;
                        }
                        prev_curve_temp = temp;
                    }
                }

                if let Some(c) = &ctx {
                    c.request_repaint();
                }
            }
        }

        // Periodic battery care poll (5s)
        if last_battery.elapsed() >= Duration::from_secs(5) {
            last_battery = Instant::now();
            if let Ok(bat) = battery_be.get_battery_status() {
                let conflict_active = state
                    .lock()
                    .map(|s| s.hp_charge_service_running)
                    .unwrap_or(false);
                if cfg.battery.enabled && !conflict_active {
                    let target = battery_engine.evaluate(
                        &mut cfg.battery,
                        bat.charge_percent,
                        bat.ac_connected,
                    );
                    if last_applied_bat_mode != Some(target) {
                        if let Err(err) = battery_be.set_charge_mode(target) {
                            tracing::debug!("Auto battery limit skipped: {err}");
                        } else {
                            last_applied_bat_mode = Some(target);
                        }
                    }
                }
                if let Ok(mut st) = state.lock() {
                    st.battery_status = Some(bat);
                }
            }
        }

        // BIOS 2-minute timeout keep-alive guard
        let ka_secs = cfg.thermal_fan.keep_alive_interval_secs.max(15);
        if cfg.thermal_fan.keep_alive_enabled
            && last_keep_alive.elapsed() >= Duration::from_secs(ka_secs)
        {
            last_keep_alive = Instant::now();
            let _ = thermal_be.keep_alive_tick();
        }

        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(not(windows))]
fn run_worker_loop(
    _cfg: AppConfig,
    _state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    _ctx: Option<egui::Context>,
) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, WorkerCommand::Shutdown) {
            return;
        }
    }
}

fn record_result(state: &Arc<Mutex<SharedState>>, res: Result<()>, ok_msg: String) {
    if let Ok(mut st) = state.lock() {
        match res {
            Ok(()) => st.set_toast(ok_msg, false),
            Err(err) => {
                tracing::warn!("Action failed: {err}");
                st.set_toast(err.to_string(), true);
            }
        }
    }
}
