use omvix_core::auto_profile::AutoProfileEngine;
use omvix_core::battery_care::BatteryCareEngine;
use omvix_core::thermal::{
    ThermalGuardDecision, ThermalSafetyGuard, effective_fan_curve_temp_c,
    is_system_cool_for_zero_rpm,
};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryLimitBackend, BatteryServiceInfo, BatteryStatus,
    ConflictServiceInfo, CpuBoostMode, CpuTuningBackend, DiagnosticReport, DiagnosticsBackend,
    DisplayBackend, DisplayModeInfo, FanMode, GpuMode, GpuSwitchBackend, InstalledCopyStatus,
    KeyboardBackend, KeyboardType, PerformanceMode, Result, RgbColor, SecurityStatus,
    TelemetryBackend, TelemetrySnapshot, ThermalFanBackend,
};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
use omvix_windows::{
    WindowsBatteryBackend, WindowsCpuTuningBackend, WindowsDiagnosticsBackend,
    WindowsDisplayBackend, WindowsGpuSwitchBackend, WindowsKeyboardBackend,
    WindowsTelemetryBackend, WindowsThermalFanBackend, system_info, wmi_bios::HpWmiBios,
};

const MAX_HISTORY_SAMPLES: usize = 180;

#[derive(Debug, Clone, Copy)]
pub struct MetricStats {
    pub min: f32,
    pub max: f32,
    pub sum: f64,
    pub count: u64,
}

impl Default for MetricStats {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricStats {
    pub fn new() -> Self {
        Self {
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            sum: 0.0,
            count: 0,
        }
    }

    pub fn record(&mut self, value: f32) {
        if !value.is_finite() {
            return;
        }
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        self.sum += value as f64;
        self.count += 1;
    }

    pub fn min_opt(&self) -> Option<f32> {
        (self.count > 0 && self.min.is_finite()).then_some(self.min)
    }

    pub fn max_opt(&self) -> Option<f32> {
        (self.count > 0 && self.max.is_finite()).then_some(self.max)
    }

    pub fn avg_opt(&self) -> Option<f32> {
        (self.count > 0).then_some((self.sum / self.count as f64) as f32)
    }
}

#[derive(Debug, Clone)]
pub struct SharedState {
    pub telemetry: TelemetrySnapshot,
    pub cpu_usage_history: VecDeque<(f64, f32)>,
    pub gpu_load_history: VecDeque<(f64, f32)>,
    pub ram_usage_history: VecDeque<(f64, f32)>,
    pub cpu_temp_history: VecDeque<(f64, f32)>,
    pub chassis_temp_history: VecDeque<(f64, f32)>,
    pub gpu_temp_history: VecDeque<(f64, f32)>,
    pub cpu_power_history: VecDeque<(f64, f32)>,
    pub gpu_power_history: VecDeque<(f64, f32)>,
    pub battery_power_history: VecDeque<(f64, f32)>,
    pub cpu_temp_stats: MetricStats,
    pub gpu_temp_stats: MetricStats,
    pub cpu_power_stats: MetricStats,
    pub gpu_power_stats: MetricStats,
    pub cpu_usage_stats: MetricStats,
    pub gpu_usage_stats: MetricStats,
    pub telemetry_paused: bool,
    pub thermal_guard_active: bool,
    pub pawnio_installing: bool,
    pub pawnio_cpu_active: bool,
    pub installed_copy_status: InstalledCopyStatus,
    pub battery_service: BatteryServiceInfo,
    pub battery_service_busy: bool,
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
            gpu_load_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            ram_usage_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            cpu_temp_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            chassis_temp_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            gpu_temp_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            cpu_power_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            gpu_power_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            battery_power_history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            cpu_temp_stats: MetricStats::new(),
            gpu_temp_stats: MetricStats::new(),
            cpu_power_stats: MetricStats::new(),
            gpu_power_stats: MetricStats::new(),
            cpu_usage_stats: MetricStats::new(),
            gpu_usage_stats: MetricStats::new(),
            telemetry_paused: false,
            thermal_guard_active: false,
            pawnio_installing: false,
            pawnio_cpu_active: false,
            installed_copy_status: InstalledCopyStatus::default(),
            battery_service: BatteryServiceInfo::default(),
            battery_service_busy: false,
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

    pub fn clear_telemetry_history(&mut self) {
        self.cpu_usage_history.clear();
        self.gpu_load_history.clear();
        self.ram_usage_history.clear();
        self.cpu_temp_history.clear();
        self.chassis_temp_history.clear();
        self.gpu_temp_history.clear();
        self.cpu_power_history.clear();
        self.gpu_power_history.clear();
        self.battery_power_history.clear();
        self.cpu_temp_stats = MetricStats::new();
        self.gpu_temp_stats = MetricStats::new();
        self.cpu_power_stats = MetricStats::new();
        self.gpu_power_stats = MetricStats::new();
        self.cpu_usage_stats = MetricStats::new();
        self.gpu_usage_stats = MetricStats::new();
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
    SetTelemetryIntervalMs(u64),
    SetTelemetryPaused(bool),
    ResetTelemetryStats,
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
    InstallPawnIo,
    ReloadPawnIo,
    SystemSuspending(Sender<()>),
    SystemShuttingDown(Sender<()>),
    SystemResumed,
    InstallBatteryService,
    StartBatteryService,
    StopBatteryService,
    RestartBatteryService,
    UninstallBatteryServiceZeroTrace,
    RefreshBatteryService,
    SetAutostart(bool),
    UpdateInstalledCopy,
    LaunchCustomCommand {
        command: String,
        args: String,
    },
    RestoreAllDefaults,
    RefreshDiagnostics,
    ExportDiagnostics(PathBuf),
    Shutdown,
}

struct AliveDropGuard(Arc<AtomicBool>);

impl Drop for AliveDropGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub struct BackgroundWorker {
    tx: Sender<WorkerCommand>,
    state: Arc<Mutex<SharedState>>,
    alive: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl BackgroundWorker {
    pub fn spawn(
        initial_config: AppConfig,
        ctx: Option<egui::Context>,
        window_visible: Arc<AtomicBool>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        let mut initial_state = SharedState::default();
        #[cfg(windows)]
        {
            // Populate Admin elevation synchronously (<0.01ms) so Frame 1 shows accurate badge immediately.
            initial_state.security.is_admin = system_info::is_process_elevated();
        }
        let state = Arc::new(Mutex::new(initial_state));
        let alive = Arc::new(AtomicBool::new(true));
        let state_bg = Arc::clone(&state);
        let alive_bg = Arc::clone(&alive);
        let tx_bg = tx.clone();

        let join = thread::spawn(move || {
            let _alive_guard = AliveDropGuard(alive_bg);
            run_worker_loop(initial_config, state_bg, rx, tx_bg, ctx, window_visible);
        });

        Self {
            tx,
            state,
            alive,
            join: Some(join),
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub fn send(&self, cmd: WorkerCommand) {
        let _ = self.tx.send(cmd);
    }

    pub fn sender_clone(&self) -> Sender<WorkerCommand> {
        self.tx.clone()
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

fn request_repaint_if_visible(ctx: &Option<egui::Context>, window_visible: &Arc<AtomicBool>) {
    if window_visible.load(Ordering::Relaxed) {
        if let Some(c) = ctx {
            c.request_repaint();
        }
    }
}

fn cool_hint_from_snapshot(snap: &TelemetrySnapshot) -> bool {
    is_system_cool_for_zero_rpm(snap.cpu_temp_c, snap.gpu_temp_c, snap.chassis_temp_c)
}

fn current_cool_hint(state: &Arc<Mutex<SharedState>>) -> bool {
    state
        .lock()
        .map(|st| cool_hint_from_snapshot(&st.telemetry))
        .unwrap_or(false)
}

#[cfg(windows)]
fn run_worker_loop(
    mut cfg: AppConfig,
    state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    self_tx: Sender<WorkerCommand>,
    ctx: Option<egui::Context>,
    window_visible: Arc<AtomicBool>,
) {
    // Hold a single persistent COM/WMI session on this worker thread for its entire lifetime.
    let _wmi_scope = HpWmiBios::enable_thread_cache();

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

    let started = Instant::now();

    // Phase 1 discovery: instant live telemetry + display modes + CPU boost
    {
        let snap = telemetry_be.read_telemetry().ok();
        let disp = display_be.get_display_mode().ok();
        let boost = tuning_be.read_cpu_boost_mode().ok();
        if let Ok(mut st) = state.lock() {
            if let Some(s) = snap {
                record_snapshot(&mut st, &s, started.elapsed().as_secs_f64());
            }
            st.display_mode = disp;
            st.cpu_boost = boost;
            st.pawnio_cpu_active = telemetry_be.is_pawnio_active();
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    // Phase 2 discovery: autostart integrity, battery, background service, keyboard type, GPU MUX mode
    {
        let is_admin = system_info::is_process_elevated();
        if is_admin {
            if let Ok(exe) = std::env::current_exe() {
                if let Err(err) = omvix_windows::autostart::verify_and_repair_autostart_task(
                    &exe,
                    cfg.general.start_with_windows,
                ) {
                    tracing::warn!("Autostart task integrity check/repair failed: {err}");
                }
            }
        }
        let copy_status = omvix_windows::autostart::query_installed_copy_status();
        let bat = battery_be.get_battery_status().ok();
        let bat_svc = omvix_windows::battery_service::query_service_info();
        let kbd_type = kbd_be.detect_keyboard_type().unwrap_or(KeyboardType::Unknown);
        let gpu_mode = if is_admin {
            gpu_be.get_gpu_mode().ok()
        } else {
            None
        };
        if let Ok(mut st) = state.lock() {
            st.installed_copy_status = copy_status;
            st.battery_status = bat;
            st.battery_service = bat_svc;
            st.keyboard_type = kbd_type;
            st.gpu_mode = gpu_mode;
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    // Phase 3 discovery: VBS/HVCI security & conflicting background services
    {
        let sec = system_info::collect_security_status();
        let conflicts = system_info::scan_conflict_services();
        let hp_charge_running = conflicts.iter().any(|s| {
            s.service_name.eq_ignore_ascii_case("hp-charge-limit-rs")
                && s.status.eq_ignore_ascii_case("Running")
        });
        if let Ok(mut st) = state.lock() {
            st.security = sec;
            st.conflict_services = conflicts;
            st.hp_charge_service_running = hp_charge_running;
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    // Phase 4 discovery: full read-only diagnostics + WMI probe table
    {
        let diag = diag_be.collect_diagnostics().ok();
        if let Ok(mut st) = state.lock() {
            st.diagnostics = diag;
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    let mut last_telemetry = Instant::now();
    let mut last_battery = Instant::now();
    let mut last_guard_eval = Instant::now();
    let mut last_curve_eval = Instant::now();
    let mut last_keep_alive = Instant::now();
    let mut prev_curve_temp: f32 = 45.0;
    let mut prev_curve_speed: u8 = 40;
    let mut last_applied_bat_mode: Option<BatteryChargeMode> = None;
    let mut thermal_guard = ThermalSafetyGuard::new();

    loop {
        let interval_ms = cfg.general.telemetry_interval_ms.clamp(100, 10_000);
        let sleep_ms = (interval_ms / 4).clamp(20, 100);

        match rx.recv_timeout(Duration::from_millis(sleep_ms)) {
            Ok(first_cmd) => {
                let mut cmds = vec![first_cmd];
                while let Ok(more) = rx.try_recv() {
                    cmds.push(more);
                }
                for cmd in coalesce_worker_commands(cmds) {
                    let should_shutdown = handle_worker_command(
                        cmd,
                        &mut cfg,
                        &state,
                        &self_tx,
                        &ctx,
                        &window_visible,
                        &diag_be,
                        &telemetry_be,
                        &thermal_be,
                        &battery_be,
                        &kbd_be,
                        &gpu_be,
                        &display_be,
                        &tuning_be,
                        &mut battery_engine,
                        &mut thermal_guard,
                        &mut last_applied_bat_mode,
                        &mut prev_curve_temp,
                        &mut prev_curve_speed,
                    );
                    if should_shutdown {
                        return;
                    }
                }
                request_repaint_if_visible(&ctx, &window_visible);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        // Periodic telemetry poll (throttled to >= 1.0s & Optimus-safe when hidden in tray)
        let interval_ms = cfg.general.telemetry_interval_ms.clamp(100, 10_000);
        let visible = window_visible.load(Ordering::Relaxed);
        let effective_interval_ms = if visible {
            interval_ms
        } else {
            interval_ms.max(1000)
        };
        if last_telemetry.elapsed() >= Duration::from_millis(effective_interval_ms) {
            last_telemetry = Instant::now();
            let on_battery = state
                .lock()
                .ok()
                .and_then(|s| s.telemetry.ac_connected)
                == Some(false);
            if let Ok(snap) =
                telemetry_be.read_telemetry_with_options(visible, visible || !on_battery)
            {
                let t = started.elapsed().as_secs_f64();
                let pawnio_active = telemetry_be.is_pawnio_active();
                if let Ok(mut st) = state.lock() {
                    if snap.cpu_temp_c.is_some() || pawnio_active {
                        st.pawnio_cpu_active = pawnio_active;
                    }
                    if !st.telemetry_paused {
                        record_snapshot(&mut st, &snap, t);
                    } else {
                        st.telemetry = snap.clone();
                    }
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

                // Evaluate thermal safety guard at >= 1s intervals (matching EC poll rate)
                if last_guard_eval.elapsed() >= Duration::from_millis(950) {
                    last_guard_eval = Instant::now();
                    match thermal_guard.evaluate(
                        snap.cpu_temp_c,
                        snap.gpu_temp_c,
                        snap.chassis_temp_c,
                        snap.cpu_power_w,
                        &snap.fan_rpms,
                    ) {
                        ThermalGuardDecision::EngageEmergencyMax(reason) => {
                            tracing::warn!(%reason, "Thermal safety guard engaging emergency Max Fan");
                            let _ = thermal_be.set_fan_mode(FanMode::Max, None);
                            if let Ok(mut st) = state.lock() {
                                st.thermal_guard_active = true;
                                st.set_toast(format!("Thermal Guard: {reason} -> Max Fan"), true);
                            }
                        }
                        ThermalGuardDecision::ReleaseEmergency => {
                            tracing::info!("Thermal safety guard releasing emergency Max Fan");
                            let is_cool = cool_hint_from_snapshot(&snap);
                            let _ = thermal_be.set_fan_mode_with_cool_hint(
                                cfg.thermal_fan.fan_mode,
                                Some(cfg.thermal_fan.manual_speed_pct),
                                is_cool,
                            );
                            if let Ok(mut st) = state.lock() {
                                st.thermal_guard_active = false;
                                st.set_toast("Thermal Guard released", false);
                            }
                        }
                        ThermalGuardDecision::NoChange => {}
                    }
                }

                // Evaluate custom fan curve (every 2s) when emergency guard is not overriding
                if !thermal_guard.is_active()
                    && cfg.thermal_fan.fan_mode == FanMode::CustomCurve
                    && last_curve_eval.elapsed() >= Duration::from_secs(2)
                {
                    last_curve_eval = Instant::now();
                    if let Some(temp) = effective_fan_curve_temp_c(
                        snap.cpu_temp_c,
                        snap.gpu_temp_c,
                        snap.chassis_temp_c,
                        snap.cpu_power_w,
                    ) {
                        let target_pct = cfg.thermal_fan.custom_curve.evaluate(
                            temp,
                            prev_curve_temp,
                            prev_curve_speed,
                        );
                        let is_cool = cool_hint_from_snapshot(&snap);
                        if target_pct != prev_curve_speed {
                            let _ = thermal_be.set_fan_mode_with_cool_hint(
                                FanMode::CustomCurve,
                                Some(target_pct),
                                is_cool,
                            );
                            prev_curve_speed = target_pct;
                            prev_curve_temp = temp;
                        } else if temp > prev_curve_temp {
                            // Track rising peaks without shifting baseline down while hysteresis holds speed
                            prev_curve_temp = temp;
                        }
                    }
                }

                request_repaint_if_visible(&ctx, &window_visible);
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
                let own_svc_running = omvix_windows::battery_service::is_service_running();
                if cfg.battery.enabled && !conflict_active && !own_svc_running {
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
                request_repaint_if_visible(&ctx, &window_visible);
            }
        }

        // BIOS 2-minute timeout keep-alive guard
        let ka_secs = cfg.thermal_fan.keep_alive_interval_secs.max(15);
        if cfg.thermal_fan.keep_alive_enabled
            && last_keep_alive.elapsed() >= Duration::from_secs(ka_secs)
        {
            last_keep_alive = Instant::now();
            let is_cool = current_cool_hint(&state);
            let _ = thermal_be.keep_alive_tick_with_cool_hint(is_cool);
        }
    }
}

fn record_snapshot(st: &mut SharedState, snap: &TelemetrySnapshot, t: f64) {
    if let Some(u) = snap.cpu_usage_pct {
        push_sample(&mut st.cpu_usage_history, t, u);
        st.cpu_usage_stats.record(u);
    }
    if let Some(gl) = snap.gpu_usage_pct {
        push_sample(&mut st.gpu_load_history, t, gl);
        st.gpu_usage_stats.record(gl);
    }
    if let (Some(used), Some(total)) = (snap.ram_used_gb, snap.ram_total_gb) {
        if total > 0.0 {
            let pct = ((used / total) * 100.0).clamp(0.0, 100.0);
            push_sample(&mut st.ram_usage_history, t, pct);
        }
    }
    if let Some(ct) = snap.cpu_temp_c {
        push_sample(&mut st.cpu_temp_history, t, ct);
        st.cpu_temp_stats.record(ct);
    }
    if let Some(ch) = snap.chassis_temp_c {
        push_sample(&mut st.chassis_temp_history, t, ch);
    }
    if let Some(gt) = snap.gpu_temp_c {
        push_sample(&mut st.gpu_temp_history, t, gt);
        st.gpu_temp_stats.record(gt);
    }
    if let Some(cp) = snap.cpu_power_w {
        push_sample(&mut st.cpu_power_history, t, cp);
        st.cpu_power_stats.record(cp);
    }
    if let Some(gp) = snap.gpu_power_w {
        push_sample(&mut st.gpu_power_history, t, gp);
        st.gpu_power_stats.record(gp);
    }
    if let Some(bp) = snap.battery_rate_w {
        push_sample(&mut st.battery_power_history, t, bp.abs());
    }
    st.telemetry = snap.clone();
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn handle_worker_command(
    cmd: WorkerCommand,
    cfg: &mut AppConfig,
    state: &Arc<Mutex<SharedState>>,
    self_tx: &Sender<WorkerCommand>,
    ctx: &Option<egui::Context>,
    window_visible: &Arc<AtomicBool>,
    diag_be: &WindowsDiagnosticsBackend,
    telemetry_be: &WindowsTelemetryBackend,
    thermal_be: &WindowsThermalFanBackend,
    battery_be: &WindowsBatteryBackend,
    kbd_be: &WindowsKeyboardBackend,
    gpu_be: &WindowsGpuSwitchBackend,
    display_be: &WindowsDisplayBackend,
    tuning_be: &WindowsCpuTuningBackend,
    battery_engine: &mut BatteryCareEngine,
    thermal_guard: &mut ThermalSafetyGuard,
    last_applied_bat_mode: &mut Option<BatteryChargeMode>,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
) -> bool {
    match cmd {
        WorkerCommand::Shutdown => {
            tracing::info!("Background worker shutting down");
            apply_off_state_safety(
                cfg,
                thermal_guard,
                thermal_be,
                battery_be,
                "clean shutdown",
            );
            return true;
        }
        WorkerCommand::SystemSuspending(ack) => {
            tracing::info!("System suspending -> applying Fan Auto & Battery off-state safety");
            apply_off_state_safety(
                cfg,
                thermal_guard,
                thermal_be,
                battery_be,
                "system suspend",
            );
            let _ = ack.send(());
        }
        WorkerCommand::SystemShuttingDown(ack) => {
            tracing::info!("System shutting down -> applying Fan Auto & Battery off-state safety");
            apply_off_state_safety(
                cfg,
                thermal_guard,
                thermal_be,
                battery_be,
                "system shutdown",
            );
            let _ = ack.send(());
        }
        WorkerCommand::SystemResumed => {
            tracing::info!("System resumed from sleep/standby -> restoring Omvix hardware state");
            *thermal_guard = ThermalSafetyGuard::new();
            *last_applied_bat_mode = None;
            let _ = thermal_be.set_performance_mode(cfg.thermal_fan.performance_mode);

            let snap = state
                .lock()
                .map(|st| st.telemetry.clone())
                .unwrap_or_default();
            let is_cool = cool_hint_from_snapshot(&snap);
            let target_pct = resolve_fan_target_pct(
                cfg,
                &snap,
                cfg.thermal_fan.fan_mode,
                Some(cfg.thermal_fan.manual_speed_pct),
                prev_curve_temp,
                prev_curve_speed,
            );
            let _ = thermal_be.set_fan_mode_with_cool_hint(
                cfg.thermal_fan.fan_mode,
                target_pct,
                is_cool,
            );
            if cfg.lighting.enabled {
                let _ = kbd_be.set_backlight(
                    true,
                    cfg.lighting.brightness_pct,
                    &cfg.lighting.zones,
                );
            }
            if let Ok(mut st) = state.lock() {
                st.thermal_guard_active = false;
            }
            tracing::info!("Re-applied thermal, fan, and lighting state after system resume");
        }
        WorkerCommand::UpdateConfig(new_cfg) => {
            let curve_changed =
                cfg.thermal_fan.custom_curve.points != new_cfg.thermal_fan.custom_curve.points;
            let bat_changed = cfg.battery.enabled != new_cfg.battery.enabled
                || cfg.battery.mode != new_cfg.battery.mode
                || cfg.battery.stop_threshold_pct != new_cfg.battery.stop_threshold_pct
                || cfg.battery.start_threshold_pct != new_cfg.battery.start_threshold_pct
                || cfg.battery.hold_when_shutdown != new_cfg.battery.hold_when_shutdown
                || cfg.battery.one_time_full_charge != new_cfg.battery.one_time_full_charge;
            *cfg = new_cfg;
            if bat_changed {
                if let Err(err) =
                    omvix_windows::battery_service::sync_config_if_installed(&cfg.battery)
                {
                    tracing::debug!("Failed syncing battery service config: {err}");
                }
            }
            if curve_changed
                && cfg.thermal_fan.fan_mode == FanMode::CustomCurve
                && !thermal_guard.is_active()
            {
                let snap = state
                    .lock()
                    .map(|st| st.telemetry.clone())
                    .unwrap_or_default();
                let is_cool = cool_hint_from_snapshot(&snap);
                let target_pct = resolve_fan_target_pct(
                    cfg,
                    &snap,
                    FanMode::CustomCurve,
                    Some(cfg.thermal_fan.manual_speed_pct),
                    prev_curve_temp,
                    prev_curve_speed,
                );
                let _ = thermal_be.set_fan_mode_with_cool_hint(
                    FanMode::CustomCurve,
                    target_pct,
                    is_cool,
                );
            }
        }
        WorkerCommand::SetTelemetryIntervalMs(ms) => {
            cfg.general.telemetry_interval_ms = ms.clamp(100, 10_000);
        }
        WorkerCommand::SetTelemetryPaused(paused) => {
            if let Ok(mut st) = state.lock() {
                st.telemetry_paused = paused;
            }
        }
        WorkerCommand::ResetTelemetryStats => {
            if let Ok(mut st) = state.lock() {
                st.clear_telemetry_history();
            }
        }
        WorkerCommand::SetPerformanceMode(mode) => {
            cfg.thermal_fan.performance_mode = mode;
            let res = thermal_be.set_performance_mode(mode);
            record_result(state, res, format!("Performance mode -> {mode:?}"));
        }
        WorkerCommand::SetFanMode {
            mode,
            manual_speed_pct,
        } => {
            cfg.thermal_fan.fan_mode = mode;
            if let Some(pct) = manual_speed_pct {
                cfg.thermal_fan.manual_speed_pct = pct;
            }
            let snap = state
                .lock()
                .map(|st| st.telemetry.clone())
                .unwrap_or_default();
            let is_cool = cool_hint_from_snapshot(&snap);
            let target_pct = resolve_fan_target_pct(
                cfg,
                &snap,
                mode,
                manual_speed_pct,
                prev_curve_temp,
                prev_curve_speed,
            );
            let res = thermal_be.set_fan_mode_with_cool_hint(mode, target_pct, is_cool);
            record_result(state, res, format!("Fan mode -> {mode:?}"));
        }
        WorkerCommand::ApplyBatteryNow => {
            let _ = omvix_windows::battery_service::sync_config_if_installed(&cfg.battery);
            if let Ok(bat) = battery_be.get_battery_status() {
                let target = battery_engine.evaluate(
                    &mut cfg.battery,
                    bat.charge_percent,
                    bat.ac_connected,
                );
                if bat.current_mode == target && *last_applied_bat_mode == Some(target) {
                    if let Ok(mut st) = state.lock() {
                        st.battery_status = Some(bat);
                        st.set_toast(format!("Battery charge mode -> {target:?}"), false);
                    }
                } else {
                    let res = battery_be.set_charge_mode(target);
                    if res.is_ok() {
                        *last_applied_bat_mode = Some(target);
                    }
                    if let Ok(refreshed) = battery_be.get_battery_status() {
                        if let Ok(mut st) = state.lock() {
                            st.battery_status = Some(refreshed);
                        }
                    }
                    record_result(state, res, format!("Battery charge mode -> {target:?}"));
                }
            }
        }
        WorkerCommand::ApplyLighting => {
            let res = kbd_be.set_backlight(
                cfg.lighting.enabled,
                cfg.lighting.brightness_pct,
                &cfg.lighting.zones,
            );
            record_result(state, res, "Keyboard lighting updated".to_string());
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
            record_result(state, res, format!("Refresh rate -> {hz} Hz"));
        }
        WorkerCommand::SetGpuMode(mode) => {
            let res = gpu_be.set_gpu_mode(mode);
            if res.is_ok() {
                if let Ok(mut st) = state.lock() {
                    st.gpu_mode = Some(mode);
                }
            }
            record_result(
                state,
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
            record_result(state, res, format!("CPU Boost -> {mode:?}"));
        }
        WorkerCommand::SetPowerLimits { pl1, pl2 } => {
            let res = tuning_be.set_power_limits(pl1, pl2);
            record_result(state, res, format!("Power limits PL1={pl1:?} PL2={pl2:?}"));
        }
        WorkerCommand::SetTccLimit(temp_c) => {
            let res = tuning_be.set_tcc_temp_limit(temp_c);
            record_result(state, res, format!("TCC temp limit -> {temp_c}°C"));
        }
        WorkerCommand::SetUndervolt(mv) => {
            let res = tuning_be.set_core_undervolt_mv(mv);
            record_result(state, res, format!("Core undervolt -> {mv} mV"));
        }
        WorkerCommand::InstallPawnIo => {
            let already_running = state
                .lock()
                .map(|mut st| {
                    if st.pawnio_installing {
                        true
                    } else {
                        st.pawnio_installing = true;
                        st.set_toast("Downloading & installing signed PawnIO driver...", false);
                        false
                    }
                })
                .unwrap_or(true);
            if !already_running {
                let state_bg = Arc::clone(state);
                let tx_bg = self_tx.clone();
                let ctx_bg = ctx.clone();
                let vis_bg = Arc::clone(window_visible);
                thread::spawn(move || {
                    let res = omvix_windows::pawnio::install_pawnio_and_module();
                    match res {
                        Ok(msg) => {
                            if let Ok(mut st) = state_bg.lock() {
                                st.pawnio_installing = false;
                                st.set_toast(msg, false);
                            }
                            let _ = tx_bg.send(WorkerCommand::ReloadPawnIo);
                        }
                        Err(err) => {
                            if let Ok(mut st) = state_bg.lock() {
                                st.pawnio_installing = false;
                                st.set_toast(err.to_string(), true);
                            }
                        }
                    }
                    request_repaint_if_visible(&ctx_bg, &vis_bg);
                });
            }
        }
        WorkerCommand::ReloadPawnIo => {
            let active = telemetry_be.reload_pawnio();
            let sec = system_info::collect_security_status();
            if let Ok(mut st) = state.lock() {
                st.pawnio_cpu_active = active;
                st.security = sec;
                if active {
                    st.set_toast("PawnIO CPU die temperature sensor active", false);
                } else {
                    st.set_toast(
                        "PawnIO installed; reboot may be needed to activate sensor",
                        true,
                    );
                }
            }
        }
        WorkerCommand::InstallBatteryService => {
            let bat_cfg = cfg.battery.clone();
            spawn_battery_service_task(
                state,
                self_tx,
                ctx,
                window_visible,
                "Installing / updating OmvixBatteryService...",
                move || omvix_windows::battery_service::install_or_update_service(&bat_cfg),
            );
        }
        WorkerCommand::StartBatteryService => {
            spawn_battery_service_task(
                state,
                self_tx,
                ctx,
                window_visible,
                "Starting OmvixBatteryService...",
                omvix_windows::battery_service::start_service,
            );
        }
        WorkerCommand::StopBatteryService => {
            spawn_battery_service_task(
                state,
                self_tx,
                ctx,
                window_visible,
                "Stopping OmvixBatteryService...",
                omvix_windows::battery_service::stop_service,
            );
        }
        WorkerCommand::RestartBatteryService => {
            spawn_battery_service_task(
                state,
                self_tx,
                ctx,
                window_visible,
                "Restarting OmvixBatteryService...",
                omvix_windows::battery_service::restart_service,
            );
        }
        WorkerCommand::UninstallBatteryServiceZeroTrace => {
            spawn_battery_service_task(
                state,
                self_tx,
                ctx,
                window_visible,
                "Removing OmvixBatteryService & cleaning all traces...",
                omvix_windows::battery_service::uninstall_service_zero_trace,
            );
        }
        WorkerCommand::RefreshBatteryService => {
            *last_applied_bat_mode = None;
            let svc_info = omvix_windows::battery_service::query_service_info();
            let copy_status = omvix_windows::autostart::query_installed_copy_status();
            let bat = battery_be.get_battery_status().ok();
            if let Ok(mut st) = state.lock() {
                st.battery_service = svc_info;
                st.installed_copy_status = copy_status;
                if bat.is_some() {
                    st.battery_status = bat;
                }
            }
        }
        WorkerCommand::SetAutostart(enabled) => {
            cfg.general.start_with_windows = enabled;
            let res = std::env::current_exe()
                .map_err(Into::into)
                .and_then(|exe| omvix_windows::autostart::set_autostart(enabled, &exe));
            let copy_status = omvix_windows::autostart::query_installed_copy_status();
            if let Ok(mut st) = state.lock() {
                st.installed_copy_status = copy_status;
            }
            record_result(
                state,
                res,
                if enabled {
                    "Autostart enabled (protected copy)".to_string()
                } else {
                    "Autostart disabled".to_string()
                },
            );
        }
        WorkerCommand::UpdateInstalledCopy => {
            let res = std::env::current_exe()
                .map_err(Into::into)
                .and_then(|exe| omvix_windows::autostart::install_or_update_gui_copy(&exe))
                .and_then(|installed| {
                    if cfg.general.start_with_windows
                        || omvix_windows::autostart::is_elevated_task_autostart_enabled()
                    {
                        omvix_windows::autostart::create_elevated_task(&installed)?;
                    }
                    Ok(())
                });
            let copy_status = omvix_windows::autostart::query_installed_copy_status();
            if let Ok(mut st) = state.lock() {
                st.installed_copy_status = copy_status;
            }
            record_result(state, res, "Updated Program Files copy".to_string());
        }
        WorkerCommand::LaunchCustomCommand { command, args } => {
            let res = omvix_windows::unelevated::launch_unelevated(&command, &args);
            match res {
                Ok(msg) => {
                    if let Ok(mut st) = state.lock() {
                        st.set_toast(msg, false);
                    }
                }
                Err(err) => {
                    tracing::warn!("Custom command failed: {err}");
                    if let Ok(mut st) = state.lock() {
                        st.set_toast(err.to_string(), true);
                    }
                }
            }
        }
        WorkerCommand::RestoreAllDefaults => {
            *thermal_guard = ThermalSafetyGuard::new();
            let _ = thermal_be.set_fan_mode(FanMode::Auto, None);
            let _ = thermal_be.set_performance_mode(PerformanceMode::Balanced);
            if omvix_windows::battery_service::is_service_installed()
                || omvix_windows::battery_service::has_service_leftovers()
            {
                let _ = omvix_windows::battery_service::uninstall_service_zero_trace();
            } else {
                omvix_windows::autostart::remove_autostart_and_installed_copy();
            }
            *last_applied_bat_mode = None;
            let _ = battery_be.set_charge_mode(BatteryChargeMode::Auto);
            let _ = tuning_be.set_cpu_boost_mode(CpuBoostMode::Aggressive);
            let _ = kbd_be.set_backlight(
                true,
                100,
                &[RgbColor {
                    r: 245,
                    g: 245,
                    b: 255,
                }; 4],
            );
            let disp = display_be.get_display_mode().ok();
            if let Some(ref d) = disp {
                if let Some(&max_hz) = d.supported_hz.iter().max() {
                    let _ = display_be.set_refresh_rate(max_hz);
                }
            }
            let refreshed_disp = display_be.get_display_mode().ok();
            let refreshed_boost = tuning_be.read_cpu_boost_mode().ok();
            let refreshed_bat = battery_be.get_battery_status().ok();
            let refreshed_svc = omvix_windows::battery_service::query_service_info();
            let refreshed_copy = omvix_windows::autostart::query_installed_copy_status();
            if let Ok(mut st) = state.lock() {
                st.thermal_guard_active = false;
                st.battery_service = refreshed_svc;
                st.installed_copy_status = refreshed_copy;
                if refreshed_disp.is_some() {
                    st.display_mode = refreshed_disp;
                }
                if refreshed_boost.is_some() {
                    st.cpu_boost = refreshed_boost;
                }
                if refreshed_bat.is_some() {
                    st.battery_status = refreshed_bat;
                }
                st.set_toast("All hardware & app settings restored to defaults", false);
            }
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
                        state,
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
    false
}

#[cfg(not(windows))]
fn run_worker_loop(
    _cfg: AppConfig,
    _state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    _self_tx: Sender<WorkerCommand>,
    _ctx: Option<egui::Context>,
    _window_visible: Arc<AtomicBool>,
) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, WorkerCommand::Shutdown) {
            return;
        }
    }
}

#[cfg(windows)]
fn resolve_fan_target_pct(
    cfg: &AppConfig,
    snap: &TelemetrySnapshot,
    mode: FanMode,
    manual_speed_pct: Option<u8>,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
) -> Option<u8> {
    if mode == FanMode::CustomCurve {
        if let Some(temp) = effective_fan_curve_temp_c(
            snap.cpu_temp_c,
            snap.gpu_temp_c,
            snap.chassis_temp_c,
            snap.cpu_power_w,
        ) {
            let pct = cfg.thermal_fan.custom_curve.evaluate(temp, temp, 0);
            *prev_curve_temp = temp;
            *prev_curve_speed = pct;
            return Some(pct);
        }
    }
    manual_speed_pct
}

#[cfg(windows)]
fn apply_off_state_safety(
    cfg: &AppConfig,
    thermal_guard: &ThermalSafetyGuard,
    thermal_be: &WindowsThermalFanBackend,
    battery_be: &WindowsBatteryBackend,
    reason: &str,
) {
    if cfg.thermal_fan.fan_mode != FanMode::Auto || thermal_guard.is_active() {
        if let Err(err) = thermal_be.set_fan_mode(FanMode::Auto, None) {
            tracing::warn!(%reason, "Failed to restore BIOS Auto fan: {err}");
        } else {
            tracing::info!(%reason, "Restored BIOS Auto fan control");
        }
    }
    if !omvix_windows::battery_service::is_service_running() && cfg.battery.enabled {
        if !cfg.battery.hold_when_shutdown {
            if let Err(err) = battery_be.set_charge_mode(BatteryChargeMode::Auto) {
                tracing::warn!(%reason, "Failed to restore normal battery charging: {err}");
            } else {
                tracing::info!(%reason, "Restored normal battery charging (0x2B Auto)");
            }
        } else if let Ok(bat) = battery_be.get_battery_status() {
            if !cfg.battery.one_time_full_charge
                && bat.charge_percent >= 40
                && bat.charge_percent >= cfg.battery.start_threshold_pct
            {
                if let Err(err) = battery_be.set_charge_mode(BatteryChargeMode::Hold) {
                    tracing::warn!(%reason, "Failed to apply Hold mode: {err}");
                } else {
                    tracing::info!(%reason, "Applied battery Hold (0x0300)");
                }
            }
        }
    }
}

#[cfg(windows)]
fn spawn_battery_service_task<F>(
    state: &Arc<Mutex<SharedState>>,
    self_tx: &Sender<WorkerCommand>,
    ctx: &Option<egui::Context>,
    window_visible: &Arc<AtomicBool>,
    pending_msg: &str,
    op: F,
) where
    F: FnOnce() -> Result<String> + Send + 'static,
{
    let already_busy = state
        .lock()
        .map(|mut st| {
            if st.battery_service_busy {
                true
            } else {
                st.battery_service_busy = true;
                st.set_toast(pending_msg, false);
                false
            }
        })
        .unwrap_or(true);
    if already_busy {
        return;
    }

    let state_bg = Arc::clone(state);
    let tx_bg = self_tx.clone();
    let ctx_bg = ctx.clone();
    let vis_bg = Arc::clone(window_visible);
    thread::spawn(move || {
        let res = op();
        if let Ok(mut st) = state_bg.lock() {
            st.battery_service_busy = false;
            match res {
                Ok(msg) => st.set_toast(msg, false),
                Err(err) => st.set_toast(err.to_string(), true),
            }
        }
        let _ = tx_bg.send(WorkerCommand::RefreshBatteryService);
        request_repaint_if_visible(&ctx_bg, &vis_bg);
    });
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

fn latest_wins_kind(cmd: &WorkerCommand) -> Option<u8> {
    match cmd {
        WorkerCommand::SetPerformanceMode(_) => Some(0),
        WorkerCommand::SetFanMode { .. } => Some(1),
        WorkerCommand::ApplyLighting => Some(2),
        WorkerCommand::ApplyBatteryNow => Some(3),
        WorkerCommand::SetTelemetryIntervalMs(_) => Some(4),
        WorkerCommand::SetRefreshRate(_) => Some(5),
        WorkerCommand::SetCpuBoost(_) => Some(6),
        _ => None,
    }
}

fn is_barrier_command(cmd: &WorkerCommand) -> bool {
    matches!(
        cmd,
        WorkerCommand::Shutdown
            | WorkerCommand::SystemSuspending(_)
            | WorkerCommand::SystemShuttingDown(_)
            | WorkerCommand::SystemResumed
            | WorkerCommand::RestoreAllDefaults
            | WorkerCommand::InstallBatteryService
            | WorkerCommand::StartBatteryService
            | WorkerCommand::StopBatteryService
            | WorkerCommand::RestartBatteryService
            | WorkerCommand::UninstallBatteryServiceZeroTrace
            | WorkerCommand::InstallPawnIo
            | WorkerCommand::ReloadPawnIo
            | WorkerCommand::UpdateInstalledCopy
            | WorkerCommand::ExportDiagnostics(_)
    )
}

fn coalesce_segment(seg: Vec<WorkerCommand>, out: &mut Vec<WorkerCommand>) {
    if seg.is_empty() {
        return;
    }
    let mut last_idx_by_kind = [None::<usize>; 7];
    for (idx, cmd) in seg.iter().enumerate() {
        if let Some(kind) = latest_wins_kind(cmd) {
            last_idx_by_kind[kind as usize] = Some(idx);
        }
    }

    let start_out = out.len();
    for (idx, cmd) in seg.into_iter().enumerate() {
        if let Some(kind) = latest_wins_kind(&cmd) {
            if last_idx_by_kind[kind as usize] != Some(idx) {
                continue;
            }
        }
        let is_update_cfg = matches!(cmd, WorkerCommand::UpdateConfig(_));
        if is_update_cfg
            && out.len() > start_out
            && matches!(out.last(), Some(WorkerCommand::UpdateConfig(_)))
        {
            *out.last_mut().expect("non-empty segment") = cmd;
        } else {
            out.push(cmd);
        }
    }
}

/// Deduplicates queued burst hardware commands per barrier-free segment (keeping only the latest
/// setting for each hardware domain) while preserving ordering around barriers and consumer steps.
fn coalesce_worker_commands(cmds: Vec<WorkerCommand>) -> Vec<WorkerCommand> {
    if cmds.len() <= 1 {
        return cmds;
    }
    let mut out = Vec::with_capacity(cmds.len());
    let mut seg = Vec::new();
    for cmd in cmds {
        if is_barrier_command(&cmd) {
            coalesce_segment(std::mem::take(&mut seg), &mut out);
            out.push(cmd);
        } else {
            seg.push(cmd);
        }
    }
    coalesce_segment(seg, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_worker_commands_deduplicates_bursts_and_respects_barriers() {
        let mut cfg1 = AppConfig::default();
        cfg1.lighting.brightness_pct = 20;
        let mut cfg2 = AppConfig::default();
        cfg2.lighting.brightness_pct = 80;

        let cmds = vec![
            WorkerCommand::UpdateConfig(cfg1.clone()),
            WorkerCommand::ApplyLighting,
            WorkerCommand::UpdateConfig(cfg2.clone()),
            WorkerCommand::ApplyLighting,
            WorkerCommand::SetFanMode {
                mode: FanMode::Manual,
                manual_speed_pct: Some(30),
            },
            WorkerCommand::SetFanMode {
                mode: FanMode::Manual,
                manual_speed_pct: Some(55),
            },
            WorkerCommand::SetPowerLimits {
                pl1: Some(45),
                pl2: Some(90),
            },
            WorkerCommand::UpdateConfig(cfg1),
            WorkerCommand::RestoreAllDefaults,
            WorkerCommand::SetFanMode {
                mode: FanMode::Auto,
                manual_speed_pct: None,
            },
            WorkerCommand::SetFanMode {
                mode: FanMode::Max,
                manual_speed_pct: None,
            },
        ];

        let coalesced = coalesce_worker_commands(cmds);
        assert_eq!(coalesced.len(), 7);
        assert!(matches!(
            &coalesced[0],
            WorkerCommand::UpdateConfig(c) if c.lighting.brightness_pct == 80
        ));
        assert!(matches!(coalesced[1], WorkerCommand::ApplyLighting));
        assert!(matches!(
            coalesced[2],
            WorkerCommand::SetFanMode {
                mode: FanMode::Manual,
                manual_speed_pct: Some(55)
            }
        ));
        assert!(matches!(
            coalesced[3],
            WorkerCommand::SetPowerLimits {
                pl1: Some(45),
                pl2: Some(90)
            }
        ));
        assert!(matches!(
            &coalesced[4],
            WorkerCommand::UpdateConfig(c) if c.lighting.brightness_pct == 20
        ));
        assert!(matches!(coalesced[5], WorkerCommand::RestoreAllDefaults));
        assert!(matches!(
            coalesced[6],
            WorkerCommand::SetFanMode {
                mode: FanMode::Max,
                manual_speed_pct: None
            }
        ));
    }
}
