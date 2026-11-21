use omvix_core::app_profiles::{AppProfileEngine, AppProfileRule, AppProfileTransition, ProfileBaseline};
use omvix_core::auto_profile::{AutoProfileAction, AutoProfileEngine};
use omvix_core::battery_care::BatteryCareEngine;
use omvix_core::boards::{is_bios_write_allowed, lookup_board};
use omvix_core::config::{BatteryConfig, LightingConfig};
use omvix_core::crash_guard::{CrashGuardState, SafeModeReason, current_unix_secs};
use omvix_core::support_bundle::{SupportBundleInputs, build_support_bundle_zip};
use omvix_core::thermal::{
    DEFAULT_SENSOR_LOSS_TIMEOUT_SECS, GuardMaxAction, GuardMaxRetryController, SensorLossDecision,
    SensorLossGuard, ThermalGuardConfig, ThermalGuardDecision, ThermalSafetyGuard,
    effective_fan_curve_temp_c, is_system_cool_for_zero_rpm,
};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryLimitBackend, BatteryServiceInfo, BatteryStatus,
    ChangeDomain, ChangeHistory, ConflictServiceInfo, CpuBoostMode, CpuTuningBackend,
    DiagnosticReport, DiagnosticsBackend, DisplayBackend, DisplayModeInfo, FanCurve, FanMode,
    GpuMode, GpuSwitchBackend, HardwareChangeEntry, InstalledCopyStatus, KeyboardBackend,
    KeyboardType, PerformanceMode, Result, RevertPayload, RgbColor, SecurityStatus,
    TelemetryBackend, TelemetrySnapshot, ThermalFanBackend, WindowsPowerOverlay,
};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
use omvix_windows::{
    WindowsBatteryBackend, WindowsCpuTuningBackend, WindowsDiagnosticsBackend,
    WindowsDisplayBackend, WindowsGpuSwitchBackend, WindowsKeyboardBackend,
    WindowsTelemetryBackend, WindowsThermalFanBackend,
    notifications::{ToastSeverity, send_windows_toast, show_mode_osd},
    system_info, wmi_bios::HpWmiBios,
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
    pub safe_mode: Option<SafeModeReason>,
    pub thermal_guard_active: bool,
    pub sensor_loss_active: bool,
    pub board_id: String,
    pub board_known: bool,
    pub bios_writes_allowed: bool,
    pub fan_conflict_services: Vec<String>,
    pub gpu_mux_reboot_pending: bool,
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
    pub effective_performance_mode: PerformanceMode,
    pub effective_fan_mode: FanMode,
    pub active_app_profile: Option<String>,
    pub active_app_profile_id: Option<String>,
    pub active_win_power_mode: Option<WindowsPowerOverlay>,
    pub auto_applied_revision: u64,
    pub running_exes_snapshot: Vec<String>,
    pub change_history: Vec<HardwareChangeEntry>,
    pub synced_config_rev: u64,
    pub synced_config: Option<AppConfig>,
    pub last_support_bundle_path: Option<PathBuf>,
    pub support_bundle_busy: bool,
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
            safe_mode: None,
            thermal_guard_active: false,
            sensor_loss_active: false,
            board_id: String::new(),
            board_known: false,
            bios_writes_allowed: true,
            fan_conflict_services: Vec::new(),
            gpu_mux_reboot_pending: false,
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
            effective_performance_mode: PerformanceMode::default(),
            effective_fan_mode: FanMode::default(),
            active_app_profile: None,
            active_app_profile_id: None,
            active_win_power_mode: None,
            auto_applied_revision: 0,
            running_exes_snapshot: Vec::new(),
            change_history: Vec::new(),
            synced_config_rev: 0,
            synced_config: None,
            last_support_bundle_path: None,
            support_bundle_busy: false,
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
    ExitSafeMode,
    RebootSystem,
    RefreshDiagnostics,
    ExportDiagnostics(PathBuf),
    ApplyAutoProfileNow,
    RefreshRunningProcesses,
    UndoLastChange,
    UndoChangeById(u64),
    ClearChangeHistory,
    CreateSupportBundle(PathBuf),
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
        initial_safe_mode: Option<SafeModeReason>,
        crash_guard: Arc<Mutex<CrashGuardState>>,
        crash_guard_path: PathBuf,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        let mut initial_state = SharedState::default();
        initial_state.safe_mode = initial_safe_mode;
        initial_state.effective_performance_mode = initial_config.thermal_fan.performance_mode;
        initial_state.effective_fan_mode = initial_config.thermal_fan.fan_mode;
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
            run_worker_loop(
                initial_config,
                state_bg,
                rx,
                tx_bg,
                ctx,
                window_visible,
                initial_safe_mode,
                crash_guard,
                crash_guard_path,
            );
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

fn extract_fan_conflicts(conflicts: &[ConflictServiceInfo]) -> Vec<String> {
    conflicts
        .iter()
        .filter(|s| {
            s.status.eq_ignore_ascii_case("Running")
                && system_info::is_fan_thermal_conflict_service(&s.service_name)
        })
        .map(|s| s.service_name.clone())
        .collect()
}

#[cfg(not(windows))]
mod system_info {
    pub fn is_fan_thermal_conflict_service(_name: &str) -> bool {
        false
    }
}

fn should_run_keep_alive(
    keep_alive_enabled: bool,
    sensor_loss_active: bool,
    bios_writes_allowed: bool,
) -> bool {
    keep_alive_enabled && !sensor_loss_active && bios_writes_allowed
}

/// Evaluates R1 sensor-loss timeout and applies Auto fan fallback / user-mode restoration cleanly.
/// Returns `(decision, suppress_keep_alive)`.
#[allow(clippy::too_many_arguments)]
fn step_sensor_loss_cycle<F>(
    guard: &mut SensorLossGuard,
    has_any_temp: bool,
    now_secs: f64,
    bios_writes_allowed: bool,
    thermal_guard_active: bool,
    configured_fan_mode: FanMode,
    target_pct: Option<u8>,
    cool_hint: bool,
    mut apply_fan: F,
) -> (SensorLossDecision, bool)
where
    F: FnMut(FanMode, Option<u8>, bool) -> Result<()>,
{
    let decision = guard.evaluate(has_any_temp, now_secs);
    match decision {
        SensorLossDecision::EngageAutoFallback => {
            if bios_writes_allowed {
                if let Err(err) = apply_fan(FanMode::Auto, None, false) {
                    tracing::warn!("Sensor-loss Auto fan fallback write failed: {err}");
                }
            }
        }
        SensorLossDecision::RestoreUserMode => {
            if bios_writes_allowed && !thermal_guard_active && configured_fan_mode != FanMode::Auto
            {
                if let Err(err) = apply_fan(configured_fan_mode, target_pct, cool_hint) {
                    tracing::warn!("Failed restoring fan mode after sensor recovery: {err}");
                }
            }
        }
        SensorLossDecision::NoChange => {}
    }
    (decision, guard.is_active())
}

fn check_bios_write_allowed(
    allowed: bool,
    state: &Arc<Mutex<SharedState>>,
    action_name: &str,
) -> bool {
    if allowed {
        return true;
    }
    tracing::warn!(%action_name, "Blocked BIOS WMI write on unapproved unknown motherboard");
    if let Ok(mut st) = state.lock() {
        st.set_toast(
            format!("Blocked {action_name}: approve Experimental Mode for this board first"),
            true,
        );
    }
    false
}

fn record_hw_write_attempt(
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    path: &Path,
    recorded: &mut bool,
) {
    if *recorded {
        return;
    }
    if let Ok(mut guard) = crash_guard.lock() {
        if guard.record_hw_applied(current_unix_secs()) {
            let _ = guard.save_to_path(path);
        }
        *recorded = true;
    }
}

fn check_hw_write_allowed(
    safe_mode: Option<SafeModeReason>,
    bios_writes_allowed: bool,
    requires_bios: bool,
    state: &Arc<Mutex<SharedState>>,
    action_name: &str,
) -> bool {
    if safe_mode.is_some() {
        tracing::warn!(%action_name, "Blocked hardware write: Safe Mode is active");
        if let Ok(mut st) = state.lock() {
            st.set_toast(
                format!("Blocked {action_name}: Safe Mode is active"),
                true,
            );
        }
        return false;
    }
    if requires_bios && !bios_writes_allowed {
        return check_bios_write_allowed(false, state, action_name);
    }
    true
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_worker_loop(
    mut cfg: AppConfig,
    state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    self_tx: Sender<WorkerCommand>,
    ctx: Option<egui::Context>,
    window_visible: Arc<AtomicBool>,
    initial_safe_mode: Option<SafeModeReason>,
    crash_guard: Arc<Mutex<CrashGuardState>>,
    crash_guard_path: PathBuf,
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
    let mut app_profile_engine = AppProfileEngine::new();
    let mut active_curve_override: Option<FanCurve> = None;

    let started = Instant::now();

    // Phase 1 discovery: instant live telemetry + display modes + CPU boost + Windows power overlay
    {
        let snap = telemetry_be.read_telemetry().ok();
        let disp = display_be.get_display_mode().ok();
        let boost = tuning_be.read_cpu_boost_mode().ok();
        let win_overlay = omvix_windows::power_mode::read_windows_power_overlay();
        if let Ok(mut st) = state.lock() {
            if let Some(s) = snap {
                record_snapshot(&mut st, &s, started.elapsed().as_secs_f64());
            }
            st.display_mode = disp;
            st.cpu_boost = boost;
            st.active_win_power_mode = win_overlay;
            st.pawnio_cpu_active = telemetry_be.is_pawnio_active();
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    // Phase 2 discovery: board identity & R9 write gate, autostart integrity, battery, service, keyboard, GPU MUX
    let mut board_id = String::new();
    let mut bios_writes_allowed;
    {
        if let Ok(id) = system_info::collect_system_identity() {
            board_id = id.board_id.trim().to_string();
        }
        let board_known = lookup_board(&board_id).is_some();
        bios_writes_allowed =
            is_bios_write_allowed(&board_id, cfg.general.experimental_board_id.as_deref());

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
            st.board_id = board_id.clone();
            st.board_known = board_known;
            st.bios_writes_allowed = bios_writes_allowed;
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
        let fan_conflicts = extract_fan_conflicts(&conflicts);
        if let Ok(mut st) = state.lock() {
            st.security = sec;
            st.conflict_services = conflicts;
            st.fan_conflict_services = fan_conflicts;
            st.hp_charge_service_running = hp_charge_running;
        }
        request_repaint_if_visible(&ctx, &window_visible);
    }

    // Phase 4 discovery: full read-only diagnostics + WMI probe table
    {
        let diag = diag_be.collect_diagnostics().ok();
        if board_id.is_empty() {
            if let Some(ref d) = diag {
                let probed_id = d.identity.board_id.trim().to_string();
                if !probed_id.is_empty() {
                    board_id = probed_id;
                    let board_known = lookup_board(&board_id).is_some();
                    bios_writes_allowed = is_bios_write_allowed(
                        &board_id,
                        cfg.general.experimental_board_id.as_deref(),
                    );
                    if let Ok(mut st) = state.lock() {
                        st.board_id = board_id.clone();
                        st.board_known = board_known;
                        st.bios_writes_allowed = bios_writes_allowed;
                    }
                }
            }
        }
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
    let mut last_app_profile_poll = Instant::now();
    let mut prev_curve_temp: f32 = 45.0;
    let mut prev_curve_speed: u8 = 40;
    let mut last_applied_bat_mode: Option<BatteryChargeMode> = None;
    let mut sensor_loss_guard = SensorLossGuard::new(DEFAULT_SENSOR_LOSS_TIMEOUT_SECS);
    let mut thermal_guard = ThermalSafetyGuard::new();
    let mut retry_ctrl = GuardMaxRetryController::new();
    let mut safe_mode = initial_safe_mode;
    let mut hw_applied_recorded = false;
    let mut stabilized_recorded = false;
    let mut last_stability_check = Instant::now();
    let mut bat_limit_toast_sent = false;
    let mut change_history = ChangeHistory::default();
    let mut last_committed_battery = cfg.battery.clone();
    let mut last_committed_lighting = cfg.lighting.clone();
    let mut last_committed_power_limits = (cfg.tuning.pl1_watts, cfg.tuning.pl2_watts);

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
                        &mut auto_profile_engine,
                        &mut app_profile_engine,
                        &mut active_curve_override,
                        &mut sensor_loss_guard,
                        &mut thermal_guard,
                        &mut retry_ctrl,
                        &board_id,
                        &mut bios_writes_allowed,
                        &mut safe_mode,
                        &crash_guard,
                        &crash_guard_path,
                        &mut hw_applied_recorded,
                        &mut last_applied_bat_mode,
                        &mut prev_curve_temp,
                        &mut prev_curve_speed,
                        &mut change_history,
                        &mut last_committed_battery,
                        &mut last_committed_lighting,
                        &mut last_committed_power_limits,
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

        if hw_applied_recorded
            && !stabilized_recorded
            && last_stability_check.elapsed() >= Duration::from_secs(2)
        {
            last_stability_check = Instant::now();
            if let Ok(mut cg) = crash_guard.lock() {
                if cg.check_and_mark_stabilized(current_unix_secs()) {
                    let _ = cg.save_to_path(&crash_guard_path);
                    stabilized_recorded = true;
                    tracing::info!(
                        "Hardware session stabilized (>=60s); crash-loop guard marked healthy"
                    );
                }
            }
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

                // R1: Evaluate temperature sensor-loss safety timeout (default 6s)
                let eff_fan_mode = effective_fan_mode(&cfg, app_profile_engine.active_rule());
                let has_any_temp = snap.cpu_temp_c.is_some()
                    || snap.gpu_temp_c.is_some()
                    || snap.chassis_temp_c.is_some();
                let cool_hint = cool_hint_from_snapshot(&snap);
                let restore_pct = {
                    let active_curve = active_curve_override
                        .as_ref()
                        .unwrap_or(&cfg.thermal_fan.custom_curve);
                    if eff_fan_mode == FanMode::CustomCurve {
                        resolve_fan_target_pct_with_curve(
                            active_curve,
                            &snap,
                            FanMode::CustomCurve,
                            Some(cfg.thermal_fan.manual_speed_pct),
                            &mut prev_curve_temp,
                            &mut prev_curve_speed,
                        )
                    } else {
                        Some(cfg.thermal_fan.manual_speed_pct)
                    }
                };
                let (sl_dec, _) = step_sensor_loss_cycle(
                    &mut sensor_loss_guard,
                    has_any_temp,
                    t,
                    bios_writes_allowed && safe_mode.is_none(),
                    thermal_guard.is_active(),
                    eff_fan_mode,
                    restore_pct,
                    cool_hint,
                    |m, pct, cool| {
                        record_hw_write_attempt(
                            &crash_guard,
                            &crash_guard_path,
                            &mut hw_applied_recorded,
                        );
                        thermal_be.set_fan_mode_with_cool_hint(m, pct, cool)
                    },
                );
                match sl_dec {
                    SensorLossDecision::EngageAutoFallback => {
                        tracing::warn!(
                            timeout_secs = DEFAULT_SENSOR_LOSS_TIMEOUT_SECS,
                            "All temperature sensors missing; forcing BIOS Auto fan control"
                        );
                        send_windows_toast(
                            "Omvix Hub — Sensor Loss",
                            "Temperature sensors lost (>6s). Fans reverted to BIOS Auto mode.",
                            ToastSeverity::Warning,
                        );
                        if let Ok(mut st) = state.lock() {
                            st.sensor_loss_active = true;
                            st.set_toast(
                                "All thermal sensors lost (>6s) -> BIOS Auto fan fallback",
                                true,
                            );
                        }
                    }
                    SensorLossDecision::RestoreUserMode => {
                        tracing::info!(
                            mode = ?eff_fan_mode,
                            "Thermal sensors recovered -> restored user fan control"
                        );
                        if let Ok(mut st) = state.lock() {
                            st.sensor_loss_active = false;
                            st.set_toast("Thermal sensors recovered -> restored fan control", false);
                        }
                    }
                    SensorLossDecision::NoChange => {}
                }

                // Evaluate AC/DC Auto-Profile on transitions (skipped while Safe Mode active)
                if safe_mode.is_none() {
                    if let Some(ac) = snap.ac_connected {
                        if let Some(action) =
                            auto_profile_engine.evaluate(&cfg.auto_profile, ac, false)
                        {
                            tracing::info!(?action, "AC/DC transition triggered auto-profile");
                            apply_auto_profile_action(
                                action,
                                ac,
                                &mut cfg,
                                &state,
                                &thermal_be,
                                &display_be,
                                &kbd_be,
                                &mut app_profile_engine,
                                &mut active_curve_override,
                                &sensor_loss_guard,
                                &thermal_guard,
                                bios_writes_allowed,
                                safe_mode,
                                &crash_guard,
                                &crash_guard_path,
                                &mut hw_applied_recorded,
                                &mut prev_curve_temp,
                                &mut prev_curve_speed,
                            );
                        }
                    }
                }

                // Evaluate thermal safety guard & R3 Max-retry at >= 1s intervals
                let eff_fan_mode = effective_fan_mode(&cfg, app_profile_engine.active_rule());
                if safe_mode.is_none()
                    && !sensor_loss_guard.is_active()
                    && last_guard_eval.elapsed() >= Duration::from_millis(950)
                {
                    last_guard_eval = Instant::now();
                    let guard_cfg = ThermalGuardConfig {
                        cpu_threshold_c: cfg.thermal_fan.guard_cpu_temp_c,
                        sustain_secs: cfg.thermal_fan.guard_sustain_secs,
                    };
                    match thermal_guard.evaluate(
                        guard_cfg,
                        eff_fan_mode,
                        snap.cpu_temp_c,
                        snap.gpu_temp_c,
                        snap.chassis_temp_c,
                        snap.cpu_power_w,
                        &snap.fan_rpms,
                        t,
                    ) {
                        ThermalGuardDecision::EngageEmergencyMax(reason) => {
                            tracing::warn!(%reason, "Thermal safety guard engaging emergency Max Fan");
                            retry_ctrl.begin_engage(&snap.fan_rpms, t);
                            if bios_writes_allowed {
                                record_hw_write_attempt(
                                    &crash_guard,
                                    &crash_guard_path,
                                    &mut hw_applied_recorded,
                                );
                                let res = thermal_be.set_fan_mode(FanMode::Max, None);
                                if let Err(ref err) = res {
                                    tracing::warn!("Emergency Max Fan write failed: {err}");
                                }
                                match retry_ctrl.record_attempt_result(res.is_ok(), t) {
                                    GuardMaxAction::FallbackToAuto(msg) => {
                                        tracing::error!("{msg}");
                                        send_windows_toast(
                                            "Omvix Hub — Thermal Guard Fallback",
                                            &msg,
                                            ToastSeverity::Error,
                                        );
                                        if let Err(err) = thermal_be.set_fan_mode(FanMode::Auto, None) {
                                            tracing::error!("Guard Auto fan fallback failed: {err}");
                                        }
                                        if let Ok(mut st) = state.lock() {
                                            st.thermal_guard_active = true;
                                            st.set_toast(msg, true);
                                        }
                                    }
                                    _ => {
                                        send_windows_toast(
                                            "Omvix Hub — Thermal Guard",
                                            &format!("Emergency Max Fan active ({reason})."),
                                            ToastSeverity::Warning,
                                        );
                                        if let Ok(mut st) = state.lock() {
                                            st.thermal_guard_active = true;
                                            st.set_toast(format!("Thermal Guard: {reason} -> Max Fan"), true);
                                        }
                                    }
                                }
                            } else if let Ok(mut st) = state.lock() {
                                st.thermal_guard_active = true;
                                st.set_toast(format!("Thermal Guard ({reason}) blocked in read-only board mode"), true);
                            }
                        }
                        ThermalGuardDecision::ReleaseEmergency => {
                            tracing::info!("Thermal safety guard releasing emergency Max Fan");
                            retry_ctrl.reset();
                            if bios_writes_allowed {
                                record_hw_write_attempt(
                                    &crash_guard,
                                    &crash_guard_path,
                                    &mut hw_applied_recorded,
                                );
                                let is_cool = cool_hint_from_snapshot(&snap);
                                let rel_fan_mode =
                                    effective_fan_mode(&cfg, app_profile_engine.active_rule());
                                if let Err(err) = thermal_be.set_fan_mode_with_cool_hint(
                                    rel_fan_mode,
                                    Some(cfg.thermal_fan.manual_speed_pct),
                                    is_cool,
                                ) {
                                    tracing::warn!("Failed releasing Emergency Max Fan: {err}");
                                }
                            }
                            if let Ok(mut st) = state.lock() {
                                st.thermal_guard_active = false;
                                st.set_toast("Thermal Guard released", false);
                            }
                        }
                        ThermalGuardDecision::NoChange => {
                            if thermal_guard.is_active() && bios_writes_allowed {
                                match retry_ctrl.tick(&snap.fan_rpms, &snap.fan_max_rpms, t) {
                                    GuardMaxAction::RetryMax { attempt } => {
                                        tracing::warn!(attempt, "Retrying Emergency Max Fan command");
                                        record_hw_write_attempt(
                                            &crash_guard,
                                            &crash_guard_path,
                                            &mut hw_applied_recorded,
                                        );
                                        let res = thermal_be.set_fan_mode(FanMode::Max, None);
                                        if let Err(ref err) = res {
                                            tracing::warn!(attempt, "Max Fan retry failed: {err}");
                                        }
                                        if let GuardMaxAction::FallbackToAuto(msg) =
                                            retry_ctrl.record_attempt_result(res.is_ok(), t)
                                        {
                                            tracing::error!("{msg}");
                                            send_windows_toast(
                                                "Omvix Hub — Thermal Guard Fallback",
                                                &msg,
                                                ToastSeverity::Error,
                                            );
                                            if let Err(err) =
                                                thermal_be.set_fan_mode(FanMode::Auto, None)
                                            {
                                                tracing::error!("Guard Auto fallback failed: {err}");
                                            }
                                            if let Ok(mut st) = state.lock() {
                                                st.set_toast(msg, true);
                                            }
                                        }
                                    }
                                    GuardMaxAction::Verified => {
                                        tracing::info!(
                                            rpms = ?snap.fan_rpms,
                                            "Verified Emergency Max Fan physical RPM acceleration"
                                        );
                                    }
                                    GuardMaxAction::FallbackToAuto(msg) => {
                                        tracing::error!("{msg}");
                                        send_windows_toast(
                                            "Omvix Hub — Thermal Guard Fallback",
                                            &msg,
                                            ToastSeverity::Error,
                                        );
                                        record_hw_write_attempt(
                                            &crash_guard,
                                            &crash_guard_path,
                                            &mut hw_applied_recorded,
                                        );
                                        if let Err(err) = thermal_be.set_fan_mode(FanMode::Auto, None)
                                        {
                                            tracing::error!("Guard Auto fallback failed: {err}");
                                        }
                                        if let Ok(mut st) = state.lock() {
                                            st.set_toast(msg, true);
                                        }
                                    }
                                    GuardMaxAction::Wait => {}
                                }
                            }
                        }
                    }
                }

                // Evaluate custom fan curve (every 2s) when emergency & sensor-loss guards are inactive
                let eff_fan_mode = effective_fan_mode(&cfg, app_profile_engine.active_rule());
                if safe_mode.is_none()
                    && bios_writes_allowed
                    && !sensor_loss_guard.is_active()
                    && !thermal_guard.is_active()
                    && eff_fan_mode == FanMode::CustomCurve
                    && last_curve_eval.elapsed() >= Duration::from_secs(2)
                {
                    last_curve_eval = Instant::now();
                    if let Some(temp) = effective_fan_curve_temp_c(
                        snap.cpu_temp_c,
                        snap.gpu_temp_c,
                        snap.chassis_temp_c,
                        snap.cpu_power_w,
                    ) {
                        let active_curve = active_curve_override
                            .as_ref()
                            .unwrap_or(&cfg.thermal_fan.custom_curve);
                        let target_pct = active_curve.evaluate(
                            temp,
                            prev_curve_temp,
                            prev_curve_speed,
                        );
                        let is_cool = cool_hint_from_snapshot(&snap);
                        if target_pct != prev_curve_speed {
                            record_hw_write_attempt(
                                &crash_guard,
                                &crash_guard_path,
                                &mut hw_applied_recorded,
                            );
                            if let Err(err) = thermal_be.set_fan_mode_with_cool_hint(
                                FanMode::CustomCurve,
                                Some(target_pct),
                                is_cool,
                            ) {
                                tracing::warn!(target_pct, "Custom fan curve write failed: {err}");
                            } else {
                                prev_curve_speed = target_pct;
                                prev_curve_temp = temp;
                            }
                        } else if temp > prev_curve_temp {
                            // Track rising peaks without shifting baseline down while hysteresis holds speed
                            prev_curve_temp = temp;
                        }
                    }
                }

                request_repaint_if_visible(&ctx, &window_visible);
            }
        }

        // Evaluate app-based .exe profiles (skipped while Safe Mode active)
        let app_poll_interval =
            Duration::from_secs(cfg.app_profiles.poll_interval_secs.clamp(2, 15));
        let should_eval_apps = safe_mode.is_none()
            && ((cfg.app_profiles.enabled && !cfg.app_profiles.rules.is_empty())
                || app_profile_engine.active_rule().is_some());
        if should_eval_apps
            && ((!cfg.app_profiles.enabled && app_profile_engine.active_rule().is_some())
                || last_app_profile_poll.elapsed() >= app_poll_interval)
        {
            last_app_profile_poll = Instant::now();
            let running = if cfg.app_profiles.enabled {
                omvix_windows::processes::running_process_names_lower()
            } else {
                std::collections::HashSet::new()
            };
            let transition = app_profile_engine.evaluate(&cfg.app_profiles, &running, || {
                let st_guard = state.lock().ok();
                let refresh_hz = st_guard
                    .as_ref()
                    .and_then(|s| s.display_mode.as_ref().map(|d| d.current_hz));
                let cpu_boost_mode = st_guard
                    .as_ref()
                    .and_then(|s| s.cpu_boost)
                    .or(cfg.tuning.cpu_boost_mode);
                ProfileBaseline {
                    performance_mode: cfg.thermal_fan.performance_mode,
                    fan_mode: cfg.thermal_fan.fan_mode,
                    manual_speed_pct: cfg.thermal_fan.manual_speed_pct,
                    custom_curve: active_curve_override
                        .clone()
                        .unwrap_or_else(|| cfg.thermal_fan.custom_curve.clone()),
                    refresh_hz,
                    cpu_boost_mode,
                }
            });
            apply_app_profile_transition(
                transition,
                &mut cfg,
                &state,
                &thermal_be,
                &display_be,
                &tuning_be,
                &mut active_curve_override,
                &sensor_loss_guard,
                &thermal_guard,
                bios_writes_allowed,
                safe_mode,
                &crash_guard,
                &crash_guard_path,
                &mut hw_applied_recorded,
                &mut prev_curve_temp,
                &mut prev_curve_speed,
            );
            request_repaint_if_visible(&ctx, &window_visible);
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
                if safe_mode.is_none()
                    && bios_writes_allowed
                    && cfg.battery.enabled
                    && !conflict_active
                    && !own_svc_running
                {
                    let target = battery_engine.evaluate(
                        &mut cfg.battery,
                        bat.charge_percent,
                        bat.ac_connected,
                    );
                    if last_applied_bat_mode != Some(target) {
                        record_hw_write_attempt(
                            &crash_guard,
                            &crash_guard_path,
                            &mut hw_applied_recorded,
                        );
                        if let Err(err) = battery_be.set_charge_mode(target) {
                            tracing::debug!("Auto battery limit skipped: {err}");
                        } else {
                            last_applied_bat_mode = Some(target);
                        }
                    }
                }
                if cfg.battery.enabled
                    && !cfg.battery.one_time_full_charge
                    && bat.ac_connected
                    && bat.charge_percent >= cfg.battery.stop_threshold_pct
                {
                    if !bat_limit_toast_sent {
                        bat_limit_toast_sent = true;
                        send_windows_toast(
                            "Omvix Hub — Battery Care",
                            &format!(
                                "Battery charge limit reached ({}%). Charging paused.",
                                bat.charge_percent
                            ),
                            ToastSeverity::Info,
                        );
                    }
                } else if !cfg.battery.enabled
                    || !bat.ac_connected
                    || bat.charge_percent < cfg.battery.start_threshold_pct
                {
                    bat_limit_toast_sent = false;
                }
                if let Ok(mut st) = state.lock() {
                    st.battery_status = Some(bat);
                }
                request_repaint_if_visible(&ctx, &window_visible);
            }
        }

        // BIOS 2-minute timeout keep-alive guard (disabled during R1 sensor-loss fallback or Safe Mode)
        let ka_secs = cfg.thermal_fan.keep_alive_interval_secs.max(15);
        if should_run_keep_alive(
            cfg.thermal_fan.keep_alive_enabled,
            sensor_loss_guard.is_active(),
            bios_writes_allowed && safe_mode.is_none(),
        ) && last_keep_alive.elapsed() >= Duration::from_secs(ka_secs)
        {
            last_keep_alive = Instant::now();
            record_hw_write_attempt(&crash_guard, &crash_guard_path, &mut hw_applied_recorded);
            let is_cool = current_cool_hint(&state);
            if let Err(err) = thermal_be.keep_alive_tick_with_cool_hint(is_cool) {
                tracing::warn!("BIOS fan keep-alive tick failed: {err}");
            }
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
fn effective_fan_mode(cfg: &AppConfig, active_rule: Option<&AppProfileRule>) -> FanMode {
    active_rule
        .and_then(|r| r.fan_mode)
        .unwrap_or(cfg.thermal_fan.fan_mode)
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
    auto_profile_engine: &mut AutoProfileEngine,
    app_profile_engine: &mut AppProfileEngine,
    active_curve_override: &mut Option<FanCurve>,
    sensor_loss_guard: &mut SensorLossGuard,
    thermal_guard: &mut ThermalSafetyGuard,
    retry_ctrl: &mut GuardMaxRetryController,
    board_id: &str,
    bios_writes_allowed: &mut bool,
    safe_mode: &mut Option<SafeModeReason>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
    last_applied_bat_mode: &mut Option<BatteryChargeMode>,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
    change_history: &mut ChangeHistory,
    last_committed_battery: &mut BatteryConfig,
    last_committed_lighting: &mut LightingConfig,
    last_committed_power_limits: &mut (Option<u16>, Option<u16>),
) -> bool {
    match cmd {
        WorkerCommand::Shutdown => {
            tracing::info!("Background worker shutting down");
            apply_off_state_safety(
                cfg,
                effective_fan_mode(cfg, app_profile_engine.active_rule()),
                thermal_guard,
                thermal_be,
                battery_be,
                state,
                *bios_writes_allowed && safe_mode.is_none(),
                "clean shutdown",
            );
            if let Ok(mut cg) = crash_guard.lock() {
                cg.mark_clean_exit(current_unix_secs());
                let _ = cg.save_to_path(crash_guard_path);
            }
            return true;
        }
        WorkerCommand::ExitSafeMode => {
            *safe_mode = None;
            if let Ok(mut st) = state.lock() {
                st.safe_mode = None;
                st.set_toast("Exited Safe Mode — hardware controls enabled", false);
            }
            tracing::info!("User exited Safe Mode; hardware controls enabled");
        }
        WorkerCommand::SystemSuspending(ack) => {
            tracing::info!("System suspending -> applying Fan Auto & Battery off-state safety");
            apply_off_state_safety(
                cfg,
                effective_fan_mode(cfg, app_profile_engine.active_rule()),
                thermal_guard,
                thermal_be,
                battery_be,
                state,
                *bios_writes_allowed && safe_mode.is_none(),
                "system suspend",
            );
            let _ = ack.send(());
        }
        WorkerCommand::SystemShuttingDown(ack) => {
            tracing::info!("System shutting down -> applying Fan Auto & Battery off-state safety");
            apply_off_state_safety(
                cfg,
                effective_fan_mode(cfg, app_profile_engine.active_rule()),
                thermal_guard,
                thermal_be,
                battery_be,
                state,
                *bios_writes_allowed && safe_mode.is_none(),
                "system shutdown",
            );
            if let Ok(mut cg) = crash_guard.lock() {
                cg.mark_clean_exit(current_unix_secs());
                let _ = cg.save_to_path(crash_guard_path);
            }
            let _ = ack.send(());
        }
        WorkerCommand::SystemResumed => {
            tracing::info!("System resumed from sleep/standby -> restoring Omvix hardware state");
            thermal_guard.reset();
            sensor_loss_guard.reset();
            retry_ctrl.reset();
            *last_applied_bat_mode = None;
            let eff_perf = app_profile_engine
                .active_rule()
                .map(|r| r.performance_mode)
                .unwrap_or(cfg.thermal_fan.performance_mode);
            let eff_fan = effective_fan_mode(cfg, app_profile_engine.active_rule());
            if *bios_writes_allowed && safe_mode.is_none() {
                record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                if let Err(err) = thermal_be.set_performance_mode(eff_perf) {
                    tracing::warn!("Failed restoring performance mode on resume: {err}");
                }
                sync_win_power_mode_if_enabled(cfg, eff_perf, *safe_mode, state);

                let snap = state
                    .lock()
                    .map(|st| st.telemetry.clone())
                    .unwrap_or_default();
                let is_cool = cool_hint_from_snapshot(&snap);
                let active_curve = active_curve_override
                    .as_ref()
                    .unwrap_or(&cfg.thermal_fan.custom_curve);
                let target_pct = resolve_fan_target_pct_with_curve(
                    active_curve,
                    &snap,
                    eff_fan,
                    Some(cfg.thermal_fan.manual_speed_pct),
                    prev_curve_temp,
                    prev_curve_speed,
                );
                if let Err(err) = thermal_be.set_fan_mode_with_cool_hint(eff_fan, target_pct, is_cool)
                {
                    tracing::warn!("Failed restoring fan mode on resume: {err}");
                }
                if cfg.lighting.enabled {
                    if let Err(err) = kbd_be.set_backlight(
                        true,
                        cfg.lighting.brightness_pct,
                        &cfg.lighting.zones,
                    ) {
                        tracing::warn!("Failed restoring keyboard backlight on resume: {err}");
                    }
                }
            } else if safe_mode.is_some() {
                tracing::info!("System resumed while Safe Mode active; skipping hardware restore");
            }
            if let Ok(mut st) = state.lock() {
                st.thermal_guard_active = false;
                st.sensor_loss_active = false;
            }
            tracing::info!("Re-applied thermal, fan, and lighting state after system resume");
        }
        WorkerCommand::UpdateConfig(mut new_cfg) => {
            let _ = new_cfg.thermal_fan.custom_curve.normalize();
            let curve_changed =
                cfg.thermal_fan.custom_curve.points != new_cfg.thermal_fan.custom_curve.points;
            if curve_changed {
                *active_curve_override = None;
            }
            let bat_changed = cfg.battery.enabled != new_cfg.battery.enabled
                || cfg.battery.mode != new_cfg.battery.mode
                || cfg.battery.stop_threshold_pct != new_cfg.battery.stop_threshold_pct
                || cfg.battery.start_threshold_pct != new_cfg.battery.start_threshold_pct
                || cfg.battery.hold_when_shutdown != new_cfg.battery.hold_when_shutdown
                || cfg.battery.one_time_full_charge != new_cfg.battery.one_time_full_charge;
            let was_allowed = *bios_writes_allowed;
            *cfg = new_cfg;
            *bios_writes_allowed = is_bios_write_allowed(
                board_id,
                cfg.general.experimental_board_id.as_deref(),
            );
            if !was_allowed && *bios_writes_allowed {
                tracing::info!(%board_id, "Experimental BIOS write mode approved by user");
            }
            if let Ok(mut st) = state.lock() {
                st.bios_writes_allowed = *bios_writes_allowed;
            }
            if bat_changed && safe_mode.is_none() {
                if let Err(err) =
                    omvix_windows::battery_service::sync_config_if_installed(&cfg.battery)
                {
                    tracing::debug!("Failed syncing battery service config: {err}");
                }
            }
            let eff_fan = effective_fan_mode(cfg, app_profile_engine.active_rule());
            if safe_mode.is_none()
                && *bios_writes_allowed
                && curve_changed
                && eff_fan == FanMode::CustomCurve
                && !thermal_guard.is_active()
                && !sensor_loss_guard.is_active()
            {
                let snap = state
                    .lock()
                    .map(|st| st.telemetry.clone())
                    .unwrap_or_default();
                let is_cool = cool_hint_from_snapshot(&snap);
                let active_curve = active_curve_override
                    .as_ref()
                    .unwrap_or(&cfg.thermal_fan.custom_curve);
                let target_pct = resolve_fan_target_pct_with_curve(
                    active_curve,
                    &snap,
                    FanMode::CustomCurve,
                    Some(cfg.thermal_fan.manual_speed_pct),
                    prev_curve_temp,
                    prev_curve_speed,
                );
                record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                if let Err(err) = thermal_be.set_fan_mode_with_cool_hint(
                    FanMode::CustomCurve,
                    target_pct,
                    is_cool,
                ) {
                    tracing::warn!("Failed applying updated custom fan curve: {err}");
                }
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
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "SetPerformanceMode",
            ) {
                return false;
            }
            let prev_mode = cfg.thermal_fan.performance_mode;
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            cfg.thermal_fan.performance_mode = mode;
            let res = thermal_be.set_performance_mode(mode);
            if res.is_ok() {
                sync_win_power_mode_if_enabled(cfg, mode, *safe_mode, state);
                if let Ok(mut st) = state.lock() {
                    st.effective_performance_mode = mode;
                }
                record_history_entry(
                    change_history,
                    state,
                    ChangeDomain::PerformanceMode,
                    format!("{prev_mode:?}"),
                    format!("{mode:?}"),
                    RevertPayload::PerformanceMode(prev_mode),
                );
            }
            record_result(state, res, format!("Performance mode -> {mode:?}"));
        }
        WorkerCommand::SetFanMode {
            mode,
            manual_speed_pct,
        } => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "SetFanMode",
            ) {
                return false;
            }
            let prev_mode = cfg.thermal_fan.fan_mode;
            let prev_pct = cfg.thermal_fan.manual_speed_pct;
            let prev_curve = active_curve_override
                .clone()
                .unwrap_or_else(|| cfg.thermal_fan.custom_curve.clone());
            let old_sum = format_fan_summary(prev_mode, prev_pct);

            *active_curve_override = None;
            cfg.thermal_fan.fan_mode = mode;
            if let Some(pct) = manual_speed_pct {
                cfg.thermal_fan.manual_speed_pct = pct;
            }
            retry_ctrl.reset();
            if sensor_loss_guard.is_active() && mode != FanMode::Auto {
                if let Ok(mut st) = state.lock() {
                    st.set_toast(
                        "Thermal sensors lost: Auto fallback active until sensors recover",
                        true,
                    );
                }
                return false;
            }
            let snap = state
                .lock()
                .map(|st| st.telemetry.clone())
                .unwrap_or_default();
            let is_cool = cool_hint_from_snapshot(&snap);
            let target_pct = resolve_fan_target_pct_with_curve(
                &cfg.thermal_fan.custom_curve,
                &snap,
                mode,
                manual_speed_pct,
                prev_curve_temp,
                prev_curve_speed,
            );
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = thermal_be.set_fan_mode_with_cool_hint(mode, target_pct, is_cool);
            if res.is_ok() {
                if let Ok(mut st) = state.lock() {
                    st.effective_fan_mode = mode;
                }
                let new_sum = format_fan_summary(mode, cfg.thermal_fan.manual_speed_pct);
                record_history_entry(
                    change_history,
                    state,
                    ChangeDomain::FanMode,
                    old_sum,
                    new_sum,
                    RevertPayload::FanMode {
                        mode: prev_mode,
                        manual_speed_pct: prev_pct,
                        custom_curve: prev_curve,
                    },
                );
            }
            record_result(state, res, format!("Fan mode -> {mode:?}"));
        }
        WorkerCommand::ApplyBatteryNow => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "Battery 0x2B",
            ) {
                return false;
            }
            let _ = omvix_windows::battery_service::sync_config_if_installed(&cfg.battery);
            if let Ok(bat) = battery_be.get_battery_status() {
                let prev_bat = last_committed_battery.clone();
                let target = battery_engine.evaluate(
                    &mut cfg.battery,
                    bat.charge_percent,
                    bat.ac_connected,
                );
                if bat.current_mode == target && *last_applied_bat_mode == Some(target) {
                    record_history_entry(
                        change_history,
                        state,
                        ChangeDomain::BatteryLimit,
                        format_battery_summary(&prev_bat),
                        format_battery_summary(&cfg.battery),
                        RevertPayload::BatteryLimit {
                            enabled: prev_bat.enabled,
                            mode: prev_bat.mode,
                            stop_threshold_pct: prev_bat.stop_threshold_pct,
                            start_threshold_pct: prev_bat.start_threshold_pct,
                            hold_when_shutdown: prev_bat.hold_when_shutdown,
                            one_time_full_charge: prev_bat.one_time_full_charge,
                            active_profile_id: prev_bat.active_profile_id,
                        },
                    );
                    *last_committed_battery = cfg.battery.clone();
                    if let Ok(mut st) = state.lock() {
                        st.battery_status = Some(bat);
                        st.set_toast(format!("Battery charge mode -> {target:?}"), false);
                    }
                } else {
                    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                    let res = battery_be.set_charge_mode(target);
                    if res.is_ok() {
                        *last_applied_bat_mode = Some(target);
                        record_history_entry(
                            change_history,
                            state,
                            ChangeDomain::BatteryLimit,
                            format_battery_summary(&prev_bat),
                            format_battery_summary(&cfg.battery),
                            RevertPayload::BatteryLimit {
                                enabled: prev_bat.enabled,
                                mode: prev_bat.mode,
                                stop_threshold_pct: prev_bat.stop_threshold_pct,
                                start_threshold_pct: prev_bat.start_threshold_pct,
                                hold_when_shutdown: prev_bat.hold_when_shutdown,
                                one_time_full_charge: prev_bat.one_time_full_charge,
                                active_profile_id: prev_bat.active_profile_id,
                            },
                        );
                        *last_committed_battery = cfg.battery.clone();
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
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "Keyboard Lighting",
            ) {
                return false;
            }
            let prev_light = last_committed_lighting.clone();
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = kbd_be.set_backlight(
                cfg.lighting.enabled,
                cfg.lighting.brightness_pct,
                &cfg.lighting.zones,
            );
            if res.is_ok() {
                record_history_entry(
                    change_history,
                    state,
                    ChangeDomain::KeyboardLighting,
                    format_lighting_summary(&prev_light),
                    format_lighting_summary(&cfg.lighting),
                    RevertPayload::KeyboardLighting {
                        enabled: prev_light.enabled,
                        brightness_pct: prev_light.brightness_pct,
                        zones: prev_light.zones,
                    },
                );
                *last_committed_lighting = cfg.lighting.clone();
            }
            record_result(state, res, "Keyboard lighting updated".to_string());
        }
        WorkerCommand::SetRefreshRate(hz) => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                false,
                state,
                "SetRefreshRate",
            ) {
                return false;
            }
            let prev_hz = state
                .lock()
                .ok()
                .and_then(|s| s.display_mode.as_ref().map(|d| d.current_hz));
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = display_be.set_refresh_rate(hz);
            if res.is_ok() {
                if let Some(old_hz) = prev_hz {
                    record_history_entry(
                        change_history,
                        state,
                        ChangeDomain::DisplayRefreshRate,
                        format!("{old_hz} Hz"),
                        format!("{hz} Hz"),
                        RevertPayload::DisplayRefreshRate(old_hz),
                    );
                }
                if let Ok(disp) = display_be.get_display_mode() {
                    if let Ok(mut st) = state.lock() {
                        st.display_mode = Some(disp);
                    }
                }
            }
            record_result(state, res, format!("Refresh rate -> {hz} Hz"));
        }
        WorkerCommand::SetGpuMode(mode) => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "GPU MUX Switch",
            ) {
                return false;
            }
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = gpu_be.set_gpu_mode(mode);
            if res.is_ok() {
                if let Ok(mut st) = state.lock() {
                    st.gpu_mode = Some(mode);
                    st.gpu_mux_reboot_pending = true;
                }
            }
            record_result(
                state,
                res,
                format!("GPU MUX -> {mode:?} (reboot required)"),
            );
        }
        WorkerCommand::SetCpuBoost(mode) => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                false,
                state,
                "SetCpuBoost",
            ) {
                return false;
            }
            let prev_boost = state
                .lock()
                .ok()
                .and_then(|s| s.cpu_boost)
                .or(cfg.tuning.cpu_boost_mode);
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = tuning_be.set_cpu_boost_mode(mode);
            if res.is_ok() {
                cfg.tuning.cpu_boost_mode = Some(mode);
                if let Ok(mut st) = state.lock() {
                    st.cpu_boost = Some(mode);
                }
                if let Some(old_boost) = prev_boost {
                    record_history_entry(
                        change_history,
                        state,
                        ChangeDomain::CpuBoost,
                        format!("{old_boost:?}"),
                        format!("{mode:?}"),
                        RevertPayload::CpuBoost(old_boost),
                    );
                }
            }
            record_result(state, res, format!("CPU Boost -> {mode:?}"));
        }
        WorkerCommand::SetPowerLimits { pl1, pl2 } => {
            if !check_hw_write_allowed(
                *safe_mode,
                *bios_writes_allowed,
                true,
                state,
                "Power Limits",
            ) {
                return false;
            }
            let prev_limits = *last_committed_power_limits;
            record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
            let res = tuning_be.set_power_limits(pl1, pl2);
            if res.is_ok() {
                cfg.tuning.pl1_watts = pl1;
                cfg.tuning.pl2_watts = pl2;
                *last_committed_power_limits = (pl1, pl2);
                if prev_limits.0.is_some() || prev_limits.1.is_some() {
                    record_history_entry(
                        change_history,
                        state,
                        ChangeDomain::PowerLimits,
                        format_power_limits_summary(prev_limits.0, prev_limits.1),
                        format_power_limits_summary(pl1, pl2),
                        RevertPayload::PowerLimits {
                            pl1_watts: prev_limits.0,
                            pl2_watts: prev_limits.1,
                        },
                    );
                }
            }
            record_result(state, res, format!("Power limits PL1={pl1:?} PL2={pl2:?}"));
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
            *safe_mode = None;
            *hw_applied_recorded = false;
            if let Ok(mut cg) = crash_guard.lock() {
                cg.clear_history();
                let _ = cg.save_to_path(crash_guard_path);
            }
            app_profile_engine.reset();
            *active_curve_override = None;
            change_history.clear();
            *last_committed_battery = BatteryConfig::default();
            *last_committed_lighting = LightingConfig::default();
            *last_committed_power_limits = (None, None);
            thermal_guard.reset();
            sensor_loss_guard.reset();
            retry_ctrl.reset();
            if *bios_writes_allowed {
                let _ = thermal_be.set_fan_mode(FanMode::Auto, None);
                let _ = thermal_be.set_performance_mode(PerformanceMode::Balanced);
            }
            let _ = omvix_windows::power_mode::set_windows_power_overlay(
                WindowsPowerOverlay::Balanced,
            );
            if omvix_windows::battery_service::is_service_installed()
                || omvix_windows::battery_service::has_service_leftovers()
            {
                let _ = omvix_windows::battery_service::uninstall_service_zero_trace();
            } else {
                omvix_windows::autostart::remove_autostart_and_installed_copy();
            }
            *last_applied_bat_mode = None;
            if *bios_writes_allowed {
                let _ = battery_be.set_charge_mode(BatteryChargeMode::Auto);
                let _ = kbd_be.set_backlight(
                    true,
                    100,
                    &[RgbColor {
                        r: 245,
                        g: 245,
                        b: 255,
                    }; 4],
                );
            }
            *bios_writes_allowed = is_bios_write_allowed(board_id, None);
            let _ = tuning_be.set_cpu_boost_mode(CpuBoostMode::Aggressive);
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
            let refreshed_overlay = omvix_windows::power_mode::read_windows_power_overlay()
                .or(Some(WindowsPowerOverlay::Balanced));
            if let Ok(mut st) = state.lock() {
                st.safe_mode = None;
                st.thermal_guard_active = false;
                st.sensor_loss_active = false;
                st.gpu_mux_reboot_pending = false;
                st.bios_writes_allowed = *bios_writes_allowed;
                st.battery_service = refreshed_svc;
                st.installed_copy_status = refreshed_copy;
                st.active_app_profile = None;
                st.active_app_profile_id = None;
                st.effective_performance_mode = PerformanceMode::Balanced;
                st.effective_fan_mode = FanMode::Auto;
                st.active_win_power_mode = refreshed_overlay;
                st.change_history.clear();
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
        WorkerCommand::RebootSystem => {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let res = std::process::Command::new(r"C:\Windows\System32\shutdown.exe")
                .args(["/r", "/t", "5", "/c", "Omvix Hub GPU MUX switch"])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(omvix_core::OmvixError::from)
                .and_then(|s| {
                    if s.success() {
                        Ok(())
                    } else {
                        Err(omvix_core::OmvixError::Config(format!(
                            "shutdown.exe exited with code {:?}",
                            s.code()
                        )))
                    }
                });
            record_result(state, res, "Windows restart scheduled in 5 seconds...".to_string());
        }
        WorkerCommand::RefreshDiagnostics => {
            let res = diag_be.collect_diagnostics();
            match res {
                Ok(report) => {
                    let fan_conflicts = extract_fan_conflicts(&report.conflict_services);
                    if let Ok(mut st) = state.lock() {
                        st.security = report.security.clone();
                        st.conflict_services = report.conflict_services.clone();
                        st.fan_conflict_services = fan_conflicts;
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
        WorkerCommand::ApplyAutoProfileNow => {
            if safe_mode.is_some() {
                if let Ok(mut st) = state.lock() {
                    st.set_toast("Blocked ApplyAutoProfileNow: Safe Mode is active", true);
                }
                return false;
            }
            let ac = state
                .lock()
                .ok()
                .and_then(|s| s.telemetry.ac_connected)
                .or_else(|| {
                    telemetry_be
                        .read_telemetry_with_options(false, false)
                        .ok()
                        .and_then(|s| s.ac_connected)
                })
                .unwrap_or(true);
            if let Some(action) = auto_profile_engine.evaluate(&cfg.auto_profile, ac, true) {
                apply_auto_profile_action(
                    action,
                    ac,
                    cfg,
                    state,
                    thermal_be,
                    display_be,
                    kbd_be,
                    app_profile_engine,
                    active_curve_override,
                    sensor_loss_guard,
                    thermal_guard,
                    *bios_writes_allowed,
                    *safe_mode,
                    crash_guard,
                    crash_guard_path,
                    hw_applied_recorded,
                    prev_curve_temp,
                    prev_curve_speed,
                );
            }
        }
        WorkerCommand::RefreshRunningProcesses => {
            let procs = omvix_windows::processes::list_user_running_exes();
            if let Ok(mut st) = state.lock() {
                st.running_exes_snapshot = procs;
            }
        }
        WorkerCommand::UndoLastChange => {
            if let Some(entry) = change_history.latest().cloned() {
                if !check_hw_write_allowed(
                    *safe_mode,
                    *bios_writes_allowed,
                    entry.domain.requires_bios_write(),
                    state,
                    "Undo",
                ) {
                    return false;
                }
                if let Some(popped) = change_history.pop_latest() {
                    apply_undo_entry(
                        popped,
                        cfg,
                        state,
                        thermal_be,
                        battery_be,
                        kbd_be,
                        display_be,
                        tuning_be,
                        battery_engine,
                        sensor_loss_guard,
                        thermal_guard,
                        retry_ctrl,
                        active_curve_override,
                        *safe_mode,
                        crash_guard,
                        crash_guard_path,
                        hw_applied_recorded,
                        last_applied_bat_mode,
                        prev_curve_temp,
                        prev_curve_speed,
                        last_committed_battery,
                        last_committed_lighting,
                        last_committed_power_limits,
                        change_history,
                    );
                }
            }
        }
        WorkerCommand::UndoChangeById(id) => {
            if let Some(entry) = change_history.peek_by_id(id).cloned() {
                if !check_hw_write_allowed(
                    *safe_mode,
                    *bios_writes_allowed,
                    entry.domain.requires_bios_write(),
                    state,
                    "Undo",
                ) {
                    return false;
                }
                if let Some(removed) = change_history.remove_by_id(id) {
                    apply_undo_entry(
                        removed,
                        cfg,
                        state,
                        thermal_be,
                        battery_be,
                        kbd_be,
                        display_be,
                        tuning_be,
                        battery_engine,
                        sensor_loss_guard,
                        thermal_guard,
                        retry_ctrl,
                        active_curve_override,
                        *safe_mode,
                        crash_guard,
                        crash_guard_path,
                        hw_applied_recorded,
                        last_applied_bat_mode,
                        prev_curve_temp,
                        prev_curve_speed,
                        last_committed_battery,
                        last_committed_lighting,
                        last_committed_power_limits,
                        change_history,
                    );
                }
            }
        }
        WorkerCommand::ClearChangeHistory => {
            change_history.clear();
            if let Ok(mut st) = state.lock() {
                st.change_history.clear();
            }
        }
        WorkerCommand::CreateSupportBundle(out_path) => {
            if let Ok(mut st) = state.lock() {
                st.support_bundle_busy = true;
                st.set_toast("Creating support bundle (.zip)...", false);
            }
            request_repaint_if_visible(ctx, window_visible);

            let diag = diag_be.collect_diagnostics().ok();
            let extra_text_files = vec![
                (
                    "logs/battery-service.log".to_string(),
                    omvix_windows::battery_service::log_file_path(),
                    256 * 1024,
                ),
                (
                    "battery-service.toml".to_string(),
                    omvix_windows::battery_service::config_file_path(),
                    64 * 1024,
                ),
            ];
            let inputs = SupportBundleInputs {
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                config: cfg.clone(),
                diagnostics: diag.clone(),
                log_dir: cfg.logging.resolved_dir(),
                crash_guard_path: Some(crash_guard_path.to_path_buf()),
                extra_text_files,
            };
            let res = build_support_bundle_zip(&out_path, &inputs);
            if let Ok(mut st) = state.lock() {
                st.support_bundle_busy = false;
                if let Some(report) = diag {
                    st.diagnostics = Some(report);
                }
                match res {
                    Ok(summary) => {
                        let kb = (summary.total_bytes / 1024).max(1);
                        st.last_support_bundle_path = Some(summary.output_path.clone());
                        st.set_toast(
                            format!(
                                "Support bundle saved ({} files, {kb} KB)",
                                summary.file_count
                            ),
                            false,
                        );
                    }
                    Err(err) => {
                        tracing::warn!("Failed creating support bundle: {err}");
                        st.set_toast(err.to_string(), true);
                    }
                }
            }
        }
    }
    false
}

#[cfg(not(windows))]
#[allow(clippy::too_many_arguments)]
fn run_worker_loop(
    _cfg: AppConfig,
    _state: Arc<Mutex<SharedState>>,
    rx: mpsc::Receiver<WorkerCommand>,
    _self_tx: Sender<WorkerCommand>,
    _ctx: Option<egui::Context>,
    _window_visible: Arc<AtomicBool>,
    _initial_safe_mode: Option<SafeModeReason>,
    _crash_guard: Arc<Mutex<CrashGuardState>>,
    _crash_guard_path: PathBuf,
) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, WorkerCommand::Shutdown) {
            return;
        }
    }
}

#[cfg(windows)]
fn resolve_fan_target_pct_with_curve(
    curve: &FanCurve,
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
            let pct = curve.evaluate(temp, temp, 0);
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
    effective_fan: FanMode,
    thermal_guard: &ThermalSafetyGuard,
    thermal_be: &WindowsThermalFanBackend,
    battery_be: &WindowsBatteryBackend,
    state: &Arc<Mutex<SharedState>>,
    bios_writes_allowed: bool,
    reason: &str,
) {
    let t_total = Instant::now();
    let mut fan_ms = 0u128;
    let mut battery_ms = 0u128;

    if bios_writes_allowed && (effective_fan != FanMode::Auto || thermal_guard.is_active()) {
        let t_fan = Instant::now();
        if let Err(err) = thermal_be.set_fan_mode(FanMode::Auto, None) {
            tracing::warn!(%reason, "Failed to restore BIOS Auto fan: {err}");
        } else {
            tracing::info!(%reason, "Restored BIOS Auto fan control");
        }
        fan_ms = t_fan.elapsed().as_millis();
    }

    if bios_writes_allowed
        && !omvix_windows::battery_service::is_service_running()
        && cfg.battery.enabled
    {
        let t_bat = Instant::now();
        if !cfg.battery.hold_when_shutdown {
            if let Err(err) = battery_be.set_charge_mode(BatteryChargeMode::Auto) {
                tracing::warn!(%reason, "Failed to restore normal battery charging: {err}");
            } else {
                tracing::info!(%reason, "Restored normal battery charging (0x2B Auto)");
            }
        } else if !cfg.battery.one_time_full_charge {
            let fast_pct = omvix_windows::telemetry::read_fast_battery_pct().or_else(|| {
                state.lock().ok().and_then(|s| {
                    s.telemetry
                        .battery_pct
                        .or_else(|| s.battery_status.as_ref().map(|b| b.charge_percent))
                })
            });
            if let Some(pct) = fast_pct {
                if pct >= 40 && pct >= cfg.battery.start_threshold_pct {
                    if let Err(err) = battery_be.set_charge_mode(BatteryChargeMode::Hold) {
                        tracing::warn!(%reason, "Failed to apply Hold mode: {err}");
                    } else {
                        tracing::info!(%reason, pct, "Applied fast battery Hold (0x0300)");
                    }
                }
            }
        }
        battery_ms = t_bat.elapsed().as_millis();
    }

    tracing::info!(
        %reason,
        fan_ms,
        battery_ms,
        total_ms = t_total.elapsed().as_millis(),
        "Completed off-state hardware safety sequence"
    );
}

#[cfg(windows)]
fn sync_win_power_mode_if_enabled(
    cfg: &AppConfig,
    mode: PerformanceMode,
    safe_mode: Option<SafeModeReason>,
    state: &Arc<Mutex<SharedState>>,
) {
    if safe_mode.is_some() || !cfg.thermal_fan.sync_windows_power_mode {
        return;
    }
    let overlay = WindowsPowerOverlay::for_performance_mode(mode);
    match omvix_windows::power_mode::set_windows_power_overlay(overlay) {
        Ok(()) => {
            if let Ok(mut st) = state.lock() {
                st.active_win_power_mode = Some(overlay);
            }
        }
        Err(err) => {
            tracing::warn!(?mode, ?overlay, "Windows Power Mode sync skipped: {err}");
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn apply_fan_override_if_safe(
    fan_mode: FanMode,
    curve: &FanCurve,
    manual_speed_pct: u8,
    state: &Arc<Mutex<SharedState>>,
    thermal_be: &WindowsThermalFanBackend,
    sensor_loss_guard: &SensorLossGuard,
    thermal_guard: &ThermalSafetyGuard,
    bios_writes_allowed: bool,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
) {
    if !bios_writes_allowed || sensor_loss_guard.is_active() || thermal_guard.is_active() {
        return;
    }
    let snap = state
        .lock()
        .map(|st| st.telemetry.clone())
        .unwrap_or_default();
    let is_cool = cool_hint_from_snapshot(&snap);
    let target_pct = resolve_fan_target_pct_with_curve(
        curve,
        &snap,
        fan_mode,
        Some(manual_speed_pct),
        prev_curve_temp,
        prev_curve_speed,
    );
    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
    if let Err(err) = thermal_be.set_fan_mode_with_cool_hint(fan_mode, target_pct, is_cool) {
        tracing::warn!(?fan_mode, "Profile fan mode write failed: {err}");
    }
}

#[cfg(windows)]
fn apply_refresh_rate_if_set(
    refresh_hz: Option<u32>,
    display_be: &WindowsDisplayBackend,
    state: &Arc<Mutex<SharedState>>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
) {
    let Some(hz) = refresh_hz else {
        return;
    };
    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
    if display_be.set_refresh_rate(hz).is_ok() {
        if let Ok(disp) = display_be.get_display_mode() {
            if let Ok(mut st) = state.lock() {
                st.display_mode = Some(disp);
            }
        }
    }
}

#[cfg(windows)]
fn apply_cpu_boost_if_set(
    cpu_boost: Option<CpuBoostMode>,
    tuning_be: &WindowsCpuTuningBackend,
    state: &Arc<Mutex<SharedState>>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
) {
    let Some(boost) = cpu_boost else {
        return;
    };
    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
    if let Err(err) = tuning_be.set_cpu_boost_mode(boost) {
        tracing::warn!(?boost, "Profile CPU boost write failed: {err}");
    } else if let Ok(mut st) = state.lock() {
        st.cpu_boost = Some(boost);
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn apply_auto_profile_action(
    action: AutoProfileAction,
    ac: bool,
    cfg: &mut AppConfig,
    state: &Arc<Mutex<SharedState>>,
    thermal_be: &WindowsThermalFanBackend,
    display_be: &WindowsDisplayBackend,
    kbd_be: &WindowsKeyboardBackend,
    app_profile_engine: &mut AppProfileEngine,
    active_curve_override: &mut Option<FanCurve>,
    sensor_loss_guard: &SensorLossGuard,
    thermal_guard: &ThermalSafetyGuard,
    bios_writes_allowed: bool,
    safe_mode: Option<SafeModeReason>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
) {
    if safe_mode.is_some() {
        return;
    }
    if app_profile_engine.active_rule().is_some() {
        app_profile_engine.update_baseline_for_ac_dc(&action);
        tracing::info!(
            ?action,
            "Deferred AC/DC auto-profile into active app-profile restore baseline"
        );
        return;
    }

    cfg.thermal_fan.performance_mode = action.performance_mode;
    let mut applied_perf = false;
    if bios_writes_allowed {
        record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
        match thermal_be.set_performance_mode(action.performance_mode) {
            Ok(()) => applied_perf = true,
            Err(err) => tracing::warn!("Auto-profile performance mode write failed: {err}"),
        }
    }
    sync_win_power_mode_if_enabled(cfg, action.performance_mode, safe_mode, state);

    if let Some(fan_mode) = action.fan_mode {
        cfg.thermal_fan.fan_mode = fan_mode;
        *active_curve_override = if fan_mode == FanMode::CustomCurve {
            action.curve_preset.map(|p| p.to_curve())
        } else {
            None
        };
        let active_curve = active_curve_override
            .as_ref()
            .unwrap_or(&cfg.thermal_fan.custom_curve);
        apply_fan_override_if_safe(
            fan_mode,
            active_curve,
            cfg.thermal_fan.manual_speed_pct,
            state,
            thermal_be,
            sensor_loss_guard,
            thermal_guard,
            bios_writes_allowed,
            crash_guard,
            crash_guard_path,
            hw_applied_recorded,
            prev_curve_temp,
            prev_curve_speed,
        );
    }

    apply_refresh_rate_if_set(
        action.refresh_hz,
        display_be,
        state,
        crash_guard,
        crash_guard_path,
        hw_applied_recorded,
    );

    if bios_writes_allowed {
        record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
        let kbd_res = if action.dim_keyboard {
            kbd_be.set_backlight(false, 0, &cfg.lighting.zones)
        } else if cfg.lighting.enabled {
            kbd_be.set_backlight(
                true,
                cfg.lighting.brightness_pct,
                &cfg.lighting.zones,
            )
        } else {
            Ok(())
        };
        if let Err(err) = kbd_res {
            tracing::warn!("Auto-profile keyboard backlight write failed: {err}");
        }
    }

    if let Ok(mut st) = state.lock() {
        st.effective_performance_mode = cfg.thermal_fan.performance_mode;
        st.effective_fan_mode = cfg.thermal_fan.fan_mode;
        st.auto_applied_revision = st.auto_applied_revision.wrapping_add(1);
    }

    if applied_perf || !bios_writes_allowed {
        let src = if ac { "AC" } else { "Battery" };
        let rgb = crate::tray::performance_mode_ring_rgb(action.performance_mode).0;
        show_mode_osd(
            &format!("Auto-Profile ({src})"),
            &format!("{:?}", action.performance_mode),
            rgb,
        );
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn apply_app_profile_transition(
    transition: AppProfileTransition,
    cfg: &mut AppConfig,
    state: &Arc<Mutex<SharedState>>,
    thermal_be: &WindowsThermalFanBackend,
    display_be: &WindowsDisplayBackend,
    tuning_be: &WindowsCpuTuningBackend,
    active_curve_override: &mut Option<FanCurve>,
    sensor_loss_guard: &SensorLossGuard,
    thermal_guard: &ThermalSafetyGuard,
    bios_writes_allowed: bool,
    safe_mode: Option<SafeModeReason>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
) {
    if safe_mode.is_some() {
        return;
    }
    match transition {
        AppProfileTransition::None => {}
        AppProfileTransition::Activate { rule } => {
            tracing::info!(
                rule_id = %rule.id,
                exe = %rule.exe_name,
                perf = ?rule.performance_mode,
                "Activating app-based hardware profile"
            );
            if bios_writes_allowed {
                record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                if let Err(err) = thermal_be.set_performance_mode(rule.performance_mode) {
                    tracing::warn!("App profile performance write failed: {err}");
                }
            }
            sync_win_power_mode_if_enabled(cfg, rule.performance_mode, safe_mode, state);

            let eff_fan = rule.fan_mode.unwrap_or(cfg.thermal_fan.fan_mode);
            *active_curve_override = if rule.fan_mode == Some(FanMode::CustomCurve) {
                rule.curve_preset.map(|p| p.to_curve())
            } else {
                None
            };
            if let Some(fan_mode) = rule.fan_mode {
                let active_curve = active_curve_override
                    .as_ref()
                    .unwrap_or(&cfg.thermal_fan.custom_curve);
                apply_fan_override_if_safe(
                    fan_mode,
                    active_curve,
                    cfg.thermal_fan.manual_speed_pct,
                    state,
                    thermal_be,
                    sensor_loss_guard,
                    thermal_guard,
                    bios_writes_allowed,
                    crash_guard,
                    crash_guard_path,
                    hw_applied_recorded,
                    prev_curve_temp,
                    prev_curve_speed,
                );
            }

            apply_refresh_rate_if_set(
                rule.refresh_hz,
                display_be,
                state,
                crash_guard,
                crash_guard_path,
                hw_applied_recorded,
            );
            apply_cpu_boost_if_set(
                rule.cpu_boost_mode,
                tuning_be,
                state,
                crash_guard,
                crash_guard_path,
                hw_applied_recorded,
            );

            let label = rule.display_label().to_string();
            if let Ok(mut st) = state.lock() {
                st.active_app_profile = Some(label.clone());
                st.active_app_profile_id = Some(rule.id.clone());
                st.effective_performance_mode = rule.performance_mode;
                st.effective_fan_mode = eff_fan;
                st.set_toast(format!("App profile active: {label}"), false);
            }
            let rgb = crate::tray::performance_mode_ring_rgb(rule.performance_mode).0;
            show_mode_osd(
                "App Profile",
                &format!("{label} · {:?}", rule.performance_mode),
                rgb,
            );
        }
        AppProfileTransition::Restore(baseline) => {
            tracing::info!(?baseline.performance_mode, ?baseline.fan_mode, "Restoring baseline after app exit");
            *active_curve_override = None;
            cfg.thermal_fan.performance_mode = baseline.performance_mode;
            cfg.thermal_fan.fan_mode = baseline.fan_mode;

            if bios_writes_allowed {
                record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                if let Err(err) = thermal_be.set_performance_mode(baseline.performance_mode) {
                    tracing::warn!("App profile baseline performance restore failed: {err}");
                }
            }
            sync_win_power_mode_if_enabled(cfg, baseline.performance_mode, safe_mode, state);

            apply_fan_override_if_safe(
                baseline.fan_mode,
                &baseline.custom_curve,
                baseline.manual_speed_pct,
                state,
                thermal_be,
                sensor_loss_guard,
                thermal_guard,
                bios_writes_allowed,
                crash_guard,
                crash_guard_path,
                hw_applied_recorded,
                prev_curve_temp,
                prev_curve_speed,
            );
            apply_refresh_rate_if_set(
                baseline.refresh_hz,
                display_be,
                state,
                crash_guard,
                crash_guard_path,
                hw_applied_recorded,
            );
            apply_cpu_boost_if_set(
                baseline.cpu_boost_mode,
                tuning_be,
                state,
                crash_guard,
                crash_guard_path,
                hw_applied_recorded,
            );

            if let Ok(mut st) = state.lock() {
                st.active_app_profile = None;
                st.active_app_profile_id = None;
                st.effective_performance_mode = baseline.performance_mode;
                st.effective_fan_mode = baseline.fan_mode;
                st.auto_applied_revision = st.auto_applied_revision.wrapping_add(1);
                st.set_toast("Restored baseline profile", false);
            }
            let rgb = crate::tray::performance_mode_ring_rgb(baseline.performance_mode).0;
            show_mode_osd(
                "Profile Restored",
                &format!("{:?}", baseline.performance_mode),
                rgb,
            );
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

#[cfg(windows)]
fn format_fan_summary(mode: FanMode, manual_pct: u8) -> String {
    match mode {
        FanMode::Manual => format!("Manual ({manual_pct}%)"),
        other => format!("{other:?}"),
    }
}

#[cfg(windows)]
fn format_battery_summary(bat: &BatteryConfig) -> String {
    if !bat.enabled || bat.mode == BatteryChargeMode::Auto {
        "Auto (100%)".to_string()
    } else if bat.one_time_full_charge {
        format!("{:?} [1x 100%]", bat.mode)
    } else {
        format!(
            "{:?} ({}-{}%)",
            bat.mode, bat.start_threshold_pct, bat.stop_threshold_pct
        )
    }
}

#[cfg(windows)]
fn format_lighting_summary(light: &LightingConfig) -> String {
    if !light.enabled {
        "Off".to_string()
    } else {
        let z0 = light.zones[0];
        format!(
            "{}% (#{:02X}{:02X}{:02X})",
            light.brightness_pct, z0.r, z0.g, z0.b
        )
    }
}

#[cfg(windows)]
fn format_power_limits_summary(pl1: Option<u16>, pl2: Option<u16>) -> String {
    let f = |v: Option<u16>| v.map(|w| format!("{w}W")).unwrap_or_else(|| "Unchanged".into());
    format!("PL1={} / PL2={}", f(pl1), f(pl2))
}

#[cfg(windows)]
fn record_history_entry(
    history: &mut ChangeHistory,
    state: &Arc<Mutex<SharedState>>,
    domain: ChangeDomain,
    old_summary: String,
    new_summary: String,
    revert: RevertPayload,
) {
    if history
        .record(domain, old_summary, new_summary, revert)
        .is_some()
    {
        if let Ok(mut st) = state.lock() {
            st.change_history = history.entries_vec();
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn apply_undo_entry(
    entry: HardwareChangeEntry,
    cfg: &mut AppConfig,
    state: &Arc<Mutex<SharedState>>,
    thermal_be: &WindowsThermalFanBackend,
    battery_be: &WindowsBatteryBackend,
    kbd_be: &WindowsKeyboardBackend,
    display_be: &WindowsDisplayBackend,
    tuning_be: &WindowsCpuTuningBackend,
    battery_engine: &mut BatteryCareEngine,
    sensor_loss_guard: &SensorLossGuard,
    thermal_guard: &ThermalSafetyGuard,
    retry_ctrl: &mut GuardMaxRetryController,
    active_curve_override: &mut Option<FanCurve>,
    safe_mode: Option<SafeModeReason>,
    crash_guard: &Arc<Mutex<CrashGuardState>>,
    crash_guard_path: &Path,
    hw_applied_recorded: &mut bool,
    last_applied_bat_mode: &mut Option<BatteryChargeMode>,
    prev_curve_temp: &mut f32,
    prev_curve_speed: &mut u8,
    last_committed_battery: &mut BatteryConfig,
    last_committed_lighting: &mut LightingConfig,
    last_committed_power_limits: &mut (Option<u16>, Option<u16>),
    change_history: &mut ChangeHistory,
) {
    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
    let res: Result<()> = match entry.revert {
        RevertPayload::PerformanceMode(mode) => {
            let r = thermal_be.set_performance_mode(mode);
            if r.is_ok() {
                cfg.thermal_fan.performance_mode = mode;
                sync_win_power_mode_if_enabled(cfg, mode, safe_mode, state);
                if let Ok(mut st) = state.lock() {
                    st.effective_performance_mode = mode;
                }
            }
            r
        }
        RevertPayload::FanMode {
            mode,
            manual_speed_pct,
            custom_curve,
        } => {
            cfg.thermal_fan.fan_mode = mode;
            cfg.thermal_fan.manual_speed_pct = manual_speed_pct;
            cfg.thermal_fan.custom_curve = custom_curve;
            *active_curve_override = None;
            retry_ctrl.reset();
            if sensor_loss_guard.is_active() && mode != FanMode::Auto {
                Err(omvix_core::OmvixError::Config(
                    "Thermal sensors lost: Auto fallback active until sensors recover".to_string(),
                ))
            } else if thermal_guard.is_active() {
                Err(omvix_core::OmvixError::Config(
                    "Thermal Guard active: Max Fan remains engaged until temperatures drop"
                        .to_string(),
                ))
            } else {
                let snap = state
                    .lock()
                    .map(|st| st.telemetry.clone())
                    .unwrap_or_default();
                let is_cool = cool_hint_from_snapshot(&snap);
                let target_pct = resolve_fan_target_pct_with_curve(
                    &cfg.thermal_fan.custom_curve,
                    &snap,
                    mode,
                    Some(manual_speed_pct),
                    prev_curve_temp,
                    prev_curve_speed,
                );
                let r = thermal_be.set_fan_mode_with_cool_hint(mode, target_pct, is_cool);
                if r.is_ok() {
                    if let Ok(mut st) = state.lock() {
                        st.effective_fan_mode = mode;
                    }
                }
                r
            }
        }
        RevertPayload::BatteryLimit {
            enabled,
            mode,
            stop_threshold_pct,
            start_threshold_pct,
            hold_when_shutdown,
            one_time_full_charge,
            active_profile_id,
        } => {
            cfg.battery.enabled = enabled;
            cfg.battery.mode = mode;
            cfg.battery.stop_threshold_pct = stop_threshold_pct;
            cfg.battery.start_threshold_pct = start_threshold_pct;
            cfg.battery.hold_when_shutdown = hold_when_shutdown;
            cfg.battery.one_time_full_charge = one_time_full_charge;
            cfg.battery.active_profile_id = active_profile_id;
            let _ = omvix_windows::battery_service::sync_config_if_installed(&cfg.battery);
            match battery_be.get_battery_status() {
                Ok(bat) => {
                    let target = battery_engine.evaluate(
                        &mut cfg.battery,
                        bat.charge_percent,
                        bat.ac_connected,
                    );
                    let r = battery_be.set_charge_mode(target);
                    if r.is_ok() {
                        *last_applied_bat_mode = Some(target);
                        *last_committed_battery = cfg.battery.clone();
                        if let Ok(refreshed) = battery_be.get_battery_status() {
                            if let Ok(mut st) = state.lock() {
                                st.battery_status = Some(refreshed);
                            }
                        }
                    }
                    r
                }
                Err(e) => Err(e),
            }
        }
        RevertPayload::DisplayRefreshRate(hz) => {
            let r = display_be.set_refresh_rate(hz);
            if r.is_ok() {
                if let Ok(disp) = display_be.get_display_mode() {
                    if let Ok(mut st) = state.lock() {
                        st.display_mode = Some(disp);
                    }
                }
            }
            r
        }
        RevertPayload::CpuBoost(boost) => {
            let r = tuning_be.set_cpu_boost_mode(boost);
            if r.is_ok() {
                cfg.tuning.cpu_boost_mode = Some(boost);
                if let Ok(mut st) = state.lock() {
                    st.cpu_boost = Some(boost);
                }
            }
            r
        }
        RevertPayload::KeyboardLighting {
            enabled,
            brightness_pct,
            zones,
        } => {
            let r = kbd_be.set_backlight(enabled, brightness_pct, &zones);
            if r.is_ok() {
                cfg.lighting.enabled = enabled;
                cfg.lighting.brightness_pct = brightness_pct;
                cfg.lighting.zones = zones;
                *last_committed_lighting = cfg.lighting.clone();
            }
            r
        }
        RevertPayload::PowerLimits {
            pl1_watts,
            pl2_watts,
        } => {
            let r = tuning_be.set_power_limits(pl1_watts, pl2_watts);
            if r.is_ok() {
                cfg.tuning.pl1_watts = pl1_watts;
                cfg.tuning.pl2_watts = pl2_watts;
                *last_committed_power_limits = (pl1_watts, pl2_watts);
            }
            r
        }
    };

    if let Ok(mut st) = state.lock() {
        st.change_history = change_history.entries_vec();
        match res {
            Ok(()) => {
                st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                st.synced_config = Some(cfg.clone());
                st.set_toast(format!("Undid change -> {}", entry.old_summary), false);
            }
            Err(err) => {
                tracing::warn!("Undo failed: {err}");
                st.set_toast(err.to_string(), true);
            }
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
            | WorkerCommand::ExitSafeMode
            | WorkerCommand::InstallBatteryService
            | WorkerCommand::StartBatteryService
            | WorkerCommand::StopBatteryService
            | WorkerCommand::RestartBatteryService
            | WorkerCommand::UninstallBatteryServiceZeroTrace
            | WorkerCommand::InstallPawnIo
            | WorkerCommand::ReloadPawnIo
            | WorkerCommand::UpdateInstalledCopy
            | WorkerCommand::RebootSystem
            | WorkerCommand::ExportDiagnostics(_)
            | WorkerCommand::UndoLastChange
            | WorkerCommand::UndoChangeById(_)
            | WorkerCommand::ClearChangeHistory
            | WorkerCommand::CreateSupportBundle(_)
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

    #[test]
    fn simulated_sensor_loss_triggers_auto_pauses_keepalive_and_restores_mode() {
        let mut sl = SensorLossGuard::new(DEFAULT_SENSOR_LOSS_TIMEOUT_SECS);
        let mut applied: Vec<(f64, FanMode, Option<u8>)> = Vec::new();
        let user_mode = FanMode::CustomCurve;
        let user_pct = Some(64u8);

        // 0..5s without any temperature reading -> stays normal, keep-alive allowed
        for sec in [0.0, 2.0, 5.0] {
            let (dec, ka_suppressed) = step_sensor_loss_cycle(
                &mut sl,
                false,
                sec,
                true,
                false,
                user_mode,
                user_pct,
                false,
                |mode, pct, _| {
                    applied.push((sec, mode, pct));
                    Ok(())
                },
            );
            assert_eq!(dec, SensorLossDecision::NoChange);
            assert!(!ka_suppressed);
            assert!(should_run_keep_alive(true, ka_suppressed, true));
        }
        assert!(applied.is_empty());

        // 6s elapsed with no temperature reading -> engages BIOS Auto & pauses keep-alive
        let (dec6, ka_suppressed6) = step_sensor_loss_cycle(
            &mut sl,
            false,
            6.0,
            true,
            false,
            user_mode,
            user_pct,
            false,
            |mode, pct, _| {
                applied.push((6.0, mode, pct));
                Ok(())
            },
        );
        assert_eq!(dec6, SensorLossDecision::EngageAutoFallback);
        assert!(ka_suppressed6);
        assert!(!should_run_keep_alive(true, ka_suppressed6, true));
        assert_eq!(applied, vec![(6.0, FanMode::Auto, None)]);

        // 8s sensor recovers -> restores user's CustomCurve & resumes keep-alive
        let (dec8, ka_suppressed8) = step_sensor_loss_cycle(
            &mut sl,
            true,
            8.0,
            true,
            false,
            user_mode,
            user_pct,
            true,
            |mode, pct, _| {
                applied.push((8.0, mode, pct));
                Ok(())
            },
        );
        assert_eq!(dec8, SensorLossDecision::RestoreUserMode);
        assert!(!ka_suppressed8);
        assert!(should_run_keep_alive(true, ka_suppressed8, true));
        assert_eq!(
            applied,
            vec![(6.0, FanMode::Auto, None), (8.0, FanMode::CustomCurve, Some(64))]
        );
    }
}
