use omvix_core::app_profiles::{
    AppProfileEngine, AppProfileRule, AppProfileTransition, ProfileBaseline,
};
use omvix_core::auto_profile::{AutoProfileAction, AutoProfileEngine};
use omvix_core::battery_care::BatteryCareEngine;
use omvix_core::battery_history::{
    battery_history_path_for, BatteryHealthHistory, BatteryHealthRecord,
};
use omvix_core::boards::{is_bios_write_allowed, lookup_board};
use omvix_core::config::{BatteryConfig, LightingConfig};
use omvix_core::crash_guard::{current_unix_secs, CrashGuardState, SafeModeReason};
use omvix_core::support_bundle::{build_support_bundle_zip, SupportBundleInputs};
use omvix_core::telemetry_csv::{write_telemetry_csv, TelemetryCsvRow, MAX_TELEMETRY_CSV_ROWS};
use omvix_core::thermal::{
    effective_fan_curve_temp_c, is_system_cool_for_zero_rpm, GuardMaxAction,
    GuardMaxRetryController, SensorLossDecision, SensorLossGuard, ThermalGuardConfig,
    ThermalGuardDecision, ThermalSafetyGuard, DEFAULT_SENSOR_LOSS_TIMEOUT_SECS,
};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryLimitBackend, BatteryServiceInfo, BatteryStatus,
    ChangeDomain, ChangeHistory, ConflictServiceInfo, CpuBoostMode, CpuTuningBackend,
    DiagnosticReport, DiagnosticsBackend, DisplayBackend, DisplayModeInfo, FanCurve, FanMode,
    GpuMode, GpuSwitchBackend, HardwareChangeEntry, InstalledCopyStatus, KeyboardBackend,
    KeyboardType, PerformanceMode, Result, RevertPayload, RgbColor, SecurityStatus,
    TelemetryBackend, TelemetrySnapshot, ThermalFanBackend, UpdateUiStatus, WindowsPowerOverlay,
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
    notifications::{send_windows_toast, show_mode_osd, ToastSeverity},
    system_info,
    wmi_bios::HpWmiBios,
    WindowsBatteryBackend, WindowsCpuTuningBackend, WindowsDiagnosticsBackend,
    WindowsDisplayBackend, WindowsGpuSwitchBackend, WindowsKeyboardBackend,
    WindowsTelemetryBackend, WindowsThermalFanBackend,
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
    pub effective_gpu_fan_mode: FanMode,
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
    pub telemetry_csv_rows: Arc<Mutex<VecDeque<TelemetryCsvRow>>>,
    pub last_csv_export_path: Option<PathBuf>,
    pub battery_health_history: Vec<BatteryHealthRecord>,
    pub update_status: UpdateUiStatus,
    pub pending_update_relaunch: Option<PathBuf>,
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
            effective_gpu_fan_mode: FanMode::default(),
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
            telemetry_csv_rows: Arc::new(Mutex::new(VecDeque::with_capacity(
                MAX_TELEMETRY_CSV_ROWS,
            ))),
            last_csv_export_path: None,
            battery_health_history: Vec::new(),
            update_status: UpdateUiStatus::Idle,
            pending_update_relaunch: None,
            last_toast: None,
        }
    }
}

impl SharedState {
    pub fn set_toast(&mut self, msg: impl Into<String>, is_error: bool) {
        self.last_toast = Some((msg.into(), is_error, Instant::now()));
    }

    pub fn has_telemetry_csv_rows(&self) -> bool {
        self.telemetry_csv_rows
            .lock()
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
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
        if let Ok(mut rows) = self.telemetry_csv_rows.lock() {
            rows.clear();
        }
        self.cpu_temp_stats = MetricStats::new();
        self.gpu_temp_stats = MetricStats::new();
        self.cpu_power_stats = MetricStats::new();
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum WorkerCommand {
    SetPerformanceMode(PerformanceMode),
    SetFanMode {
        mode: FanMode,
        gpu_mode: Option<FanMode>,
        manual_speed_pct: Option<u8>,
        gpu_manual_speed_pct: Option<u8>,
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
    RestoreDefaultPowerLimits {
        keep_pl1: Option<u16>,
        keep_pl2: Option<u16>,
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
    ExportTelemetryCsv(PathBuf),
    RecordBatteryHealthNow,
    CheckForUpdates {
        silent: bool,
    },
    DownloadAndVerifyUpdate,
    ApplyStagedUpdate,
    OpenReleasePage(String),
    PollExternalHardwareState,
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
        let mut initial_state = SharedState {
            safe_mode: initial_safe_mode,
            effective_performance_mode: initial_config.thermal_fan.performance_mode,
            effective_fan_mode: initial_config.thermal_fan.fan_mode,
            effective_gpu_fan_mode: if initial_config.thermal_fan.link_cpu_gpu_fans {
                initial_config.thermal_fan.fan_mode
            } else {
                initial_config.thermal_fan.gpu_fan_mode
            },
            ..SharedState::default()
        };
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

fn request_repaint_always(ctx: &Option<egui::Context>) {
    if let Some(c) = ctx {
        c.request_repaint();
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
    cpu_target_pct: Option<u8>,
    gpu_target_pct: Option<u8>,
    cool_hint: bool,
    mut apply_fan: F,
) -> (SensorLossDecision, bool)
where
    F: FnMut(FanMode, Option<u8>, Option<u8>, bool) -> Result<()>,
{
    let decision = guard.evaluate(has_any_temp, now_secs);
    match decision {
        SensorLossDecision::EngageAutoFallback => {
            if bios_writes_allowed {
                if let Err(err) = apply_fan(FanMode::Auto, None, None, false) {
                    tracing::warn!("Sensor-loss Auto fan fallback write failed: {err}");
                }
            }
        }
        SensorLossDecision::RestoreUserMode => {
            if bios_writes_allowed && !thermal_guard_active && configured_fan_mode != FanMode::Auto
            {
                if let Err(err) = apply_fan(
                    configured_fan_mode,
                    cpu_target_pct,
                    gpu_target_pct,
                    cool_hint,
                ) {
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
            st.set_toast(format!("Blocked {action_name}: Safe Mode is active"), true);
        }
        return false;
    }
    if requires_bios && !bios_writes_allowed {
        return check_bios_write_allowed(false, state, action_name);
    }
    true
}

trait WorkerThermalBackend: ThermalFanBackend {
    fn set_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        manual_speed_pct: Option<u8>,
        _is_cool: bool,
    ) -> Result<()> {
        self.set_fan_mode(mode, manual_speed_pct)
    }

    fn set_dual_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        cpu_speed_pct: Option<u8>,
        gpu_speed_pct: Option<u8>,
        is_cool: bool,
    ) -> Result<()> {
        self.set_fan_mode_with_cool_hint(mode, cpu_speed_pct.or(gpu_speed_pct), is_cool)
    }

    fn keep_alive_tick_with_cool_hint(&self, _is_cool: bool) -> Result<()> {
        self.keep_alive_tick()
    }
}

trait WorkerTelemetryBackend: TelemetryBackend {
    fn read_telemetry_with_options(
        &self,
        _include_io_counters: bool,
        _allow_dgpu_wake: bool,
    ) -> Result<TelemetrySnapshot> {
        self.read_telemetry()
    }

    fn is_pawnio_active(&self) -> bool {
        false
    }

    fn reload_pawnio(&self) -> bool {
        false
    }
}

trait WorkerTuningBackend: CpuTuningBackend {
    fn read_cpu_boost_mode(&self) -> Result<CpuBoostMode>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerToastSeverity {
    Info,
    Warning,
    Error,
}

trait WorkerOsHooks: Send + Sync {
    fn send_toast(&self, title: &str, body: &str, severity: WorkerToastSeverity);
    fn show_osd(&self, title: &str, value: &str, perf_mode: PerformanceMode);
    fn read_win_power_overlay(&self) -> Option<WindowsPowerOverlay>;
    fn set_win_power_overlay(&self, overlay: WindowsPowerOverlay) -> Result<()>;
    fn running_process_names_lower(&self) -> std::collections::HashSet<String>;
    fn list_user_running_exes(&self) -> Vec<String>;
    fn is_battery_service_running(&self) -> bool;
    fn read_battery_service_config(&self) -> Option<BatteryConfig>;
    fn sync_battery_service_config(&self, cfg: &BatteryConfig) -> Result<()>;
    fn read_fast_battery_pct(&self) -> Option<u8>;
}

struct WorkerBackends {
    diag: Box<dyn DiagnosticsBackend>,
    telemetry: Box<dyn WorkerTelemetryBackend>,
    thermal: Box<dyn WorkerThermalBackend>,
    battery: Box<dyn BatteryLimitBackend>,
    kbd: Box<dyn KeyboardBackend>,
    gpu: Box<dyn GpuSwitchBackend>,
    display: Box<dyn DisplayBackend>,
    tuning: Box<dyn WorkerTuningBackend>,
    os: Box<dyn WorkerOsHooks>,
}

#[cfg(windows)]
impl WorkerThermalBackend for WindowsThermalFanBackend {
    fn set_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        manual_speed_pct: Option<u8>,
        is_cool: bool,
    ) -> Result<()> {
        WindowsThermalFanBackend::set_fan_mode_with_cool_hint(self, mode, manual_speed_pct, is_cool)
    }

    fn set_dual_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        cpu_speed_pct: Option<u8>,
        gpu_speed_pct: Option<u8>,
        is_cool: bool,
    ) -> Result<()> {
        WindowsThermalFanBackend::set_dual_fan_mode_with_cool_hint(
            self,
            mode,
            cpu_speed_pct,
            gpu_speed_pct,
            is_cool,
        )
    }

    fn keep_alive_tick_with_cool_hint(&self, is_cool: bool) -> Result<()> {
        WindowsThermalFanBackend::keep_alive_tick_with_cool_hint(self, is_cool)
    }
}

#[cfg(windows)]
impl WorkerTelemetryBackend for WindowsTelemetryBackend {
    fn read_telemetry_with_options(
        &self,
        include_io_counters: bool,
        allow_dgpu_wake: bool,
    ) -> Result<TelemetrySnapshot> {
        WindowsTelemetryBackend::read_telemetry_with_options(
            self,
            include_io_counters,
            allow_dgpu_wake,
        )
    }

    fn is_pawnio_active(&self) -> bool {
        WindowsTelemetryBackend::is_pawnio_active(self)
    }

    fn reload_pawnio(&self) -> bool {
        WindowsTelemetryBackend::reload_pawnio(self)
    }
}

#[cfg(windows)]
impl WorkerTuningBackend for WindowsCpuTuningBackend {
    fn read_cpu_boost_mode(&self) -> Result<CpuBoostMode> {
        WindowsCpuTuningBackend::read_cpu_boost_mode(self)
    }
}

#[cfg(windows)]
struct WindowsOsHooks;

#[cfg(windows)]
impl WorkerOsHooks for WindowsOsHooks {
    fn send_toast(&self, title: &str, body: &str, severity: WorkerToastSeverity) {
        let mapped = match severity {
            WorkerToastSeverity::Info => ToastSeverity::Info,
            WorkerToastSeverity::Warning => ToastSeverity::Warning,
            WorkerToastSeverity::Error => ToastSeverity::Error,
        };
        send_windows_toast(title, body, mapped);
    }

    fn show_osd(&self, title: &str, value: &str, perf_mode: PerformanceMode) {
        let rgb = crate::tray::performance_mode_ring_rgb(perf_mode).0;
        show_mode_osd(title, value, rgb);
    }

    fn read_win_power_overlay(&self) -> Option<WindowsPowerOverlay> {
        omvix_windows::power_mode::read_windows_power_overlay()
    }

    fn set_win_power_overlay(&self, overlay: WindowsPowerOverlay) -> Result<()> {
        omvix_windows::power_mode::set_windows_power_overlay(overlay)
    }

    fn running_process_names_lower(&self) -> std::collections::HashSet<String> {
        omvix_windows::processes::running_process_names_lower()
    }

    fn list_user_running_exes(&self) -> Vec<String> {
        omvix_windows::processes::list_user_running_exes()
    }

    fn is_battery_service_running(&self) -> bool {
        omvix_windows::battery_service::is_service_running()
    }

    fn read_battery_service_config(&self) -> Option<BatteryConfig> {
        omvix_windows::battery_service::read_service_config().map(|c| c.to_battery_config())
    }

    fn sync_battery_service_config(&self, cfg: &BatteryConfig) -> Result<()> {
        omvix_windows::battery_service::sync_config_if_installed(cfg).map(|_| ())
    }

    fn read_fast_battery_pct(&self) -> Option<u8> {
        omvix_windows::telemetry::read_fast_battery_pct()
    }
}

#[cfg(windows)]
impl WorkerBackends {
    fn windows_default() -> Self {
        Self {
            diag: Box::new(WindowsDiagnosticsBackend::new()),
            telemetry: Box::new(WindowsTelemetryBackend::new()),
            thermal: Box::new(WindowsThermalFanBackend::new()),
            battery: Box::new(WindowsBatteryBackend::new()),
            kbd: Box::new(WindowsKeyboardBackend::new()),
            gpu: Box::new(WindowsGpuSwitchBackend::new()),
            display: Box::new(WindowsDisplayBackend::new()),
            tuning: Box::new(WindowsCpuTuningBackend::new()),
            os: Box::new(WindowsOsHooks),
        }
    }
}

#[cfg(windows)]
struct WorkerEngine {
    backends: WorkerBackends,
    cfg: AppConfig,
    state: Arc<Mutex<SharedState>>,
    battery_engine: BatteryCareEngine,
    auto_profile_engine: AutoProfileEngine,
    app_profile_engine: AppProfileEngine,
    active_curve_override: Option<FanCurve>,
    sensor_loss_guard: SensorLossGuard,
    thermal_guard: ThermalSafetyGuard,
    retry_ctrl: GuardMaxRetryController,
    board_id: String,
    bios_writes_allowed: bool,
    safe_mode: Option<SafeModeReason>,
    crash_guard: Arc<Mutex<CrashGuardState>>,
    crash_guard_path: PathBuf,
    hw_applied_recorded: bool,
    last_applied_bat_mode: Option<BatteryChargeMode>,
    prev_curve_temp: f32,
    prev_curve_speed: u8,
    prev_gpu_curve_temp: f32,
    prev_gpu_curve_speed: u8,
    prev_curve_applied: bool,
    prev_curve_cool_hint: Option<bool>,
    previous_active_rule: Option<AppProfileRule>,
    change_history: ChangeHistory,
    last_committed_battery: BatteryConfig,
    last_committed_lighting: LightingConfig,
    last_committed_power_limits: (Option<u16>, Option<u16>),
    battery_health_history: BatteryHealthHistory,
    battery_history_path: PathBuf,
    bat_limit_toast_sent: bool,
    last_kbd_write_at: Option<Instant>,
    keyboard_dimmed_by_auto_profile: bool,
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_worker_loop(
    cfg: AppConfig,
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

    let backends = WorkerBackends::windows_default();
    let started = Instant::now();
    let battery_history_path = battery_history_path_for(&crash_guard_path);
    let battery_health_history = BatteryHealthHistory::load_from_path(&battery_history_path);

    let mut engine = WorkerEngine::new(
        backends,
        cfg,
        Arc::clone(&state),
        String::new(),
        true,
        initial_safe_mode,
        crash_guard,
        crash_guard_path,
        battery_health_history,
        battery_history_path,
    );
    engine.run_startup_discovery(&self_tx, &ctx, &window_visible, &started);

    let mut last_telemetry = Instant::now();
    let mut last_battery = Instant::now();
    let mut last_guard_eval = Instant::now();
    let mut last_curve_eval = Instant::now();
    let mut last_keep_alive = Instant::now();
    let mut last_app_profile_poll = Instant::now();
    let mut last_ext_hw_poll = Instant::now();
    let mut stabilized_recorded = false;
    let mut last_stability_check = Instant::now();

    loop {
        let interval_ms = engine.cfg.general.telemetry_interval_ms.clamp(100, 10_000);
        let visible = window_visible.load(Ordering::Relaxed);
        let effective_interval_ms = if visible {
            interval_ms
        } else {
            interval_ms.max(1000)
        };
        let sleep_ms = if visible {
            (effective_interval_ms / 4).clamp(20, 100)
        } else {
            (effective_interval_ms / 2).clamp(250, 500)
        };

        match rx.recv_timeout(Duration::from_millis(sleep_ms)) {
            Ok(first_cmd) => {
                let mut cmds = vec![first_cmd];
                while let Ok(more) = rx.try_recv() {
                    cmds.push(more);
                }
                for cmd in coalesce_worker_commands(cmds) {
                    if engine.handle_command(cmd, &self_tx, &ctx, &window_visible) {
                        return;
                    }
                }
                request_repaint_if_visible(&ctx, &window_visible);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if engine.hw_applied_recorded
            && !stabilized_recorded
            && last_stability_check.elapsed() >= Duration::from_secs(2)
        {
            last_stability_check = Instant::now();
            if let Ok(mut cg) = engine.crash_guard.lock() {
                if cg.check_and_mark_stabilized(current_unix_secs()) {
                    let _ = cg.save_to_path(&engine.crash_guard_path);
                    stabilized_recorded = true;
                    tracing::info!(
                        "Hardware session stabilized (>=60s); crash-loop guard marked healthy"
                    );
                }
            }
        }

        // Periodic telemetry poll (throttled to >= 1.0s & Optimus-safe when hidden in tray)
        let interval_ms = engine.cfg.general.telemetry_interval_ms.clamp(100, 10_000);
        let visible = window_visible.load(Ordering::Relaxed);
        let effective_interval_ms = if visible {
            interval_ms
        } else {
            interval_ms.max(1000)
        };
        if last_telemetry.elapsed() >= Duration::from_millis(effective_interval_ms) {
            last_telemetry = Instant::now();
            let guard_due = last_guard_eval.elapsed() >= Duration::from_millis(950);
            let curve_due = last_curve_eval.elapsed() >= Duration::from_secs(2);
            let prev_auto_rev = engine
                .state
                .lock()
                .map(|s| s.auto_applied_revision)
                .unwrap_or(0);
            let (ran_guard, ran_curve) = engine.poll_telemetry_step(
                started.elapsed().as_secs_f64(),
                visible,
                guard_due,
                curve_due,
            );
            if ran_guard {
                last_guard_eval = Instant::now();
            }
            if ran_curve {
                last_curve_eval = Instant::now();
            }
            let new_auto_rev = engine
                .state
                .lock()
                .map(|s| s.auto_applied_revision)
                .unwrap_or(prev_auto_rev);
            if new_auto_rev != prev_auto_rev {
                request_repaint_always(&ctx);
            } else {
                request_repaint_if_visible(&ctx, &window_visible);
            }
        }

        // Evaluate app-based .exe profiles (skipped while Safe Mode active)
        let app_poll_interval =
            Duration::from_secs(engine.cfg.app_profiles.poll_interval_secs.clamp(2, 15));
        let should_eval_apps = engine.safe_mode.is_none()
            && ((engine.cfg.app_profiles.enabled && !engine.cfg.app_profiles.rules.is_empty())
                || engine.app_profile_engine.active_rule().is_some());
        if should_eval_apps
            && ((!engine.cfg.app_profiles.enabled
                && engine.app_profile_engine.active_rule().is_some())
                || last_app_profile_poll.elapsed() >= app_poll_interval)
        {
            last_app_profile_poll = Instant::now();
            if engine.poll_app_profiles_step() {
                request_repaint_always(&ctx);
            } else {
                request_repaint_if_visible(&ctx, &window_visible);
            }
        }

        // Periodic battery care poll (5s)
        if last_battery.elapsed() >= Duration::from_secs(5) {
            last_battery = Instant::now();
            let (prev_bat_mode, prev_sync_rev) = engine
                .state
                .lock()
                .map(|s| {
                    (
                        s.battery_status.as_ref().map(|b| b.current_mode),
                        s.synced_config_rev,
                    )
                })
                .unwrap_or((None, 0));
            if engine.poll_battery_step() {
                let (new_bat_mode, new_sync_rev) = engine
                    .state
                    .lock()
                    .map(|s| {
                        (
                            s.battery_status.as_ref().map(|b| b.current_mode),
                            s.synced_config_rev,
                        )
                    })
                    .unwrap_or((prev_bat_mode, prev_sync_rev));
                if new_bat_mode != prev_bat_mode || new_sync_rev != prev_sync_rev {
                    request_repaint_always(&ctx);
                } else {
                    request_repaint_if_visible(&ctx, &window_visible);
                }
            }
        }

        // Periodic external hardware sync (Fn+backlight, display rate, CPU boost, Windows power overlay)
        let ext_poll_interval = if visible {
            Duration::from_millis(2500)
        } else {
            Duration::from_secs(5)
        };
        if last_ext_hw_poll.elapsed() >= ext_poll_interval {
            last_ext_hw_poll = Instant::now();
            if engine.poll_external_hw_state_step(true) {
                request_repaint_always(&ctx);
            }
        }

        // BIOS 2-minute timeout keep-alive guard (disabled during R1 sensor-loss fallback or Safe Mode)
        let ka_secs = engine.cfg.thermal_fan.keep_alive_interval_secs.max(15);
        if last_keep_alive.elapsed() >= Duration::from_secs(ka_secs)
            && engine.poll_keep_alive_step()
        {
            last_keep_alive = Instant::now();
        }
    }
}

#[cfg(all(test, windows))]
fn spawn_test_worker(
    mut engine: WorkerEngine,
    window_visible: Arc<AtomicBool>,
) -> BackgroundWorker {
    let (tx, rx) = mpsc::channel::<WorkerCommand>();
    let state = Arc::clone(&engine.state);
    let alive = Arc::new(AtomicBool::new(true));
    let alive_bg = Arc::clone(&alive);
    let tx_bg = tx.clone();

    let join = thread::spawn(move || {
        let _alive_guard = AliveDropGuard(alive_bg);
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(first_cmd) => {
                    let mut cmds = vec![first_cmd];
                    while let Ok(more) = rx.try_recv() {
                        cmds.push(more);
                    }
                    for cmd in coalesce_worker_commands(cmds) {
                        if engine.handle_command(cmd, &tx_bg, &None, &window_visible) {
                            return;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });

    BackgroundWorker {
        tx,
        state,
        alive,
        join: Some(join),
    }
}

fn record_snapshot(st: &mut SharedState, snap: &TelemetrySnapshot, t: f64) {
    if let Some(u) = snap.cpu_usage_pct {
        push_sample(&mut st.cpu_usage_history, t, u);
    }
    if let Some(gl) = snap.gpu_usage_pct {
        push_sample(&mut st.gpu_load_history, t, gl);
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
    }
    if let Some(bp) = snap.battery_rate_w {
        push_sample(&mut st.battery_power_history, t, bp.abs());
    }
    if let Ok(mut rows) = st.telemetry_csv_rows.lock() {
        if rows.len() >= MAX_TELEMETRY_CSV_ROWS {
            rows.pop_front();
        }
        rows.push_back(TelemetryCsvRow::from_snapshot(snap, t));
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
fn effective_gpu_fan_mode(cfg: &AppConfig, active_rule: Option<&AppProfileRule>) -> FanMode {
    if let Some(m) = active_rule.and_then(|r| r.fan_mode) {
        return m;
    }
    if cfg.thermal_fan.link_cpu_gpu_fans {
        cfg.thermal_fan.fan_mode
    } else {
        cfg.thermal_fan.gpu_fan_mode
    }
}

#[cfg(windows)]
fn combined_fan_guard_mode(cpu_mode: FanMode, gpu_mode: FanMode) -> FanMode {
    if cpu_mode == FanMode::Auto && gpu_mode == FanMode::Auto {
        FanMode::Auto
    } else if cpu_mode == FanMode::Max && gpu_mode == FanMode::Max {
        FanMode::Max
    } else if cpu_mode == FanMode::CustomCurve || gpu_mode == FanMode::CustomCurve {
        FanMode::CustomCurve
    } else {
        FanMode::Manual
    }
}

#[cfg(windows)]
impl WorkerEngine {
    #[allow(clippy::too_many_arguments)]
    fn new(
        backends: WorkerBackends,
        mut cfg: AppConfig,
        state: Arc<Mutex<SharedState>>,
        board_id: String,
        check_board_gate: bool,
        safe_mode: Option<SafeModeReason>,
        crash_guard: Arc<Mutex<CrashGuardState>>,
        crash_guard_path: PathBuf,
        battery_health_history: BatteryHealthHistory,
        battery_history_path: PathBuf,
    ) -> Self {
        let _ = cfg.thermal_fan.custom_curve.normalize();
        let _ = cfg.thermal_fan.gpu_custom_curve.normalize();
        let bios_writes_allowed = if check_board_gate {
            is_bios_write_allowed(&board_id, cfg.general.experimental_board_id.as_deref())
        } else {
            true
        };
        let last_committed_battery = cfg.battery.clone();
        let last_committed_lighting = cfg.lighting.clone();
        let last_committed_power_limits = (cfg.tuning.pl1_watts, cfg.tuning.pl2_watts);

        Self {
            backends,
            cfg,
            state,
            battery_engine: BatteryCareEngine::new(),
            auto_profile_engine: AutoProfileEngine::new(),
            app_profile_engine: AppProfileEngine::new(),
            active_curve_override: None,
            sensor_loss_guard: SensorLossGuard::new(DEFAULT_SENSOR_LOSS_TIMEOUT_SECS),
            thermal_guard: ThermalSafetyGuard::new(),
            retry_ctrl: GuardMaxRetryController::new(),
            board_id,
            bios_writes_allowed,
            safe_mode,
            crash_guard,
            crash_guard_path,
            hw_applied_recorded: false,
            last_applied_bat_mode: None,
            prev_curve_temp: 45.0,
            prev_curve_speed: 40,
            prev_gpu_curve_temp: 40.0,
            prev_gpu_curve_speed: 40,
            prev_curve_applied: false,
            prev_curve_cool_hint: None,
            previous_active_rule: None,
            change_history: ChangeHistory::default(),
            last_committed_battery,
            last_committed_lighting,
            last_committed_power_limits,
            battery_health_history,
            battery_history_path,
            bat_limit_toast_sent: false,
            last_kbd_write_at: None,
            keyboard_dimmed_by_auto_profile: false,
        }
    }

    fn run_startup_discovery(
        &mut self,
        self_tx: &Sender<WorkerCommand>,
        ctx: &Option<egui::Context>,
        window_visible: &Arc<AtomicBool>,
        started: &Instant,
    ) {
        // Phase 1 discovery: instant live telemetry + display modes + CPU boost + Windows power overlay
        {
            let snap = self.backends.telemetry.read_telemetry().ok();
            let disp = self.backends.display.get_display_mode().ok();
            let boost = self.backends.tuning.read_cpu_boost_mode().ok();
            let win_overlay = self.backends.os.read_win_power_overlay();
            if let Ok(mut st) = self.state.lock() {
                if let Some(s) = snap {
                    record_snapshot(&mut st, &s, started.elapsed().as_secs_f64());
                }
                st.display_mode = disp;
                st.cpu_boost = boost;
                st.active_win_power_mode = win_overlay;
                st.pawnio_cpu_active = self.backends.telemetry.is_pawnio_active();
            }
            request_repaint_if_visible(ctx, window_visible);
        }

        // Phase 2 discovery: board identity & R9 write gate, autostart integrity, battery, service, keyboard, GPU MUX
        {
            if let Ok(id) = system_info::collect_system_identity() {
                self.board_id = id.board_id.trim().to_string();
            }
            let board_known = lookup_board(&self.board_id).is_some();
            self.bios_writes_allowed = is_bios_write_allowed(
                &self.board_id,
                self.cfg.general.experimental_board_id.as_deref(),
            );

            let is_admin = system_info::is_process_elevated();
            if is_admin {
                if let Ok(exe) = std::env::current_exe() {
                    if let Err(err) = omvix_windows::autostart::verify_and_repair_autostart_task(
                        &exe,
                        self.cfg.general.start_with_windows,
                    ) {
                        tracing::warn!("Autostart task integrity check/repair failed: {err}");
                    }
                }
            }
            let copy_status = omvix_windows::autostart::query_installed_copy_status();
            let bat = self.backends.battery.get_battery_status().ok();
            let bat_svc = omvix_windows::battery_service::query_service_info();
            if let Some(ref b) = bat {
                if let (Some(design), Some(full)) =
                    (b.design_capacity_mwh, b.full_charge_capacity_mwh)
                {
                    let date_label = chrono::Local::now().format("%Y-%m-%d").to_string();
                    if self.battery_health_history.record_if_due(
                        design,
                        full,
                        current_unix_secs() as i64,
                        date_label,
                    ) {
                        let _ = self
                            .battery_health_history
                            .save_to_path(&self.battery_history_path);
                    }
                }
            }
            let kbd_type = self
                .backends
                .kbd
                .detect_keyboard_type()
                .unwrap_or(KeyboardType::Unknown);
            let gpu_mode = if is_admin {
                self.backends.gpu.get_gpu_mode().ok()
            } else {
                None
            };
            if let Ok(mut st) = self.state.lock() {
                st.board_id = self.board_id.clone();
                st.board_known = board_known;
                st.bios_writes_allowed = self.bios_writes_allowed;
                st.installed_copy_status = copy_status;
                st.battery_status = bat;
                st.battery_service = bat_svc;
                st.battery_health_history = self.battery_health_history.records.clone();
                st.keyboard_type = kbd_type;
                st.gpu_mode = gpu_mode;
            }
            self.sync_keyboard_backlight_from_hw(false);
            self.apply_startup_saved_settings();
            request_repaint_if_visible(ctx, window_visible);
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
            if let Ok(mut st) = self.state.lock() {
                st.security = sec;
                st.conflict_services = conflicts;
                st.fan_conflict_services = fan_conflicts;
                st.hp_charge_service_running = hp_charge_running;
            }
            request_repaint_if_visible(ctx, window_visible);
        }

        // Phase 4 discovery: full read-only diagnostics + WMI probe table
        {
            let diag = self.backends.diag.collect_diagnostics().ok();
            if self.board_id.is_empty() {
                if let Some(ref d) = diag {
                    let probed_id = d.identity.board_id.trim().to_string();
                    if !probed_id.is_empty() {
                        self.board_id = probed_id;
                        let board_known = lookup_board(&self.board_id).is_some();
                        self.bios_writes_allowed = is_bios_write_allowed(
                            &self.board_id,
                            self.cfg.general.experimental_board_id.as_deref(),
                        );
                        if let Ok(mut st) = self.state.lock() {
                            st.board_id = self.board_id.clone();
                            st.board_known = board_known;
                            st.bios_writes_allowed = self.bios_writes_allowed;
                        }
                    }
                }
            }
            if let Ok(mut st) = self.state.lock() {
                st.diagnostics = diag;
            }
            request_repaint_if_visible(ctx, window_visible);
            if self.cfg.general.auto_check_updates && self.safe_mode.is_none() {
                let _ = self_tx.send(WorkerCommand::CheckForUpdates { silent: true });
            }
        }
    }

    fn poll_telemetry_step(
        &mut self,
        now_secs: f64,
        visible: bool,
        guard_due: bool,
        curve_due: bool,
    ) -> (bool, bool) {
        let on_battery = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.telemetry.ac_connected)
            == Some(false);
        let Ok(snap) = self
            .backends
            .telemetry
            .read_telemetry_with_options(visible, visible || !on_battery)
        else {
            return (false, false);
        };

        let pawnio_active = self.backends.telemetry.is_pawnio_active();
        if let Ok(mut st) = self.state.lock() {
            if snap.cpu_temp_c.is_some() || pawnio_active {
                st.pawnio_cpu_active = pawnio_active;
            }
            if !st.telemetry_paused {
                record_snapshot(&mut st, &snap, now_secs);
            } else {
                st.telemetry = snap.clone();
            }
        }

        // R1: Evaluate temperature sensor-loss safety timeout (default 6s)
        let eff_cpu_mode = effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_gpu_mode = effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_guard_mode = combined_fan_guard_mode(eff_cpu_mode, eff_gpu_mode);
        let has_any_temp =
            snap.cpu_temp_c.is_some() || snap.gpu_temp_c.is_some() || snap.chassis_temp_c.is_some();
        let cool_hint = cool_hint_from_snapshot(&snap);
        let link_fans =
            self.cfg.thermal_fan.link_cpu_gpu_fans || self.active_curve_override.is_some();
        let (restore_hw_mode, restore_cpu_pct, restore_gpu_pct) =
            if self.sensor_loss_guard.is_active() && has_any_temp {
                let active_cpu_curve = self
                    .active_curve_override
                    .as_ref()
                    .unwrap_or(&self.cfg.thermal_fan.custom_curve);
                resolve_independent_dual_fan_targets(
                    eff_cpu_mode,
                    eff_gpu_mode,
                    &snap,
                    active_cpu_curve,
                    &self.cfg.thermal_fan.gpu_custom_curve,
                    Some(self.cfg.thermal_fan.manual_speed_pct),
                    Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
                    link_fans,
                    &mut self.prev_curve_temp,
                    &mut self.prev_curve_speed,
                    &mut self.prev_gpu_curve_temp,
                    &mut self.prev_gpu_curve_speed,
                )
            } else {
                (eff_guard_mode, None, None)
            };
        let (sl_dec, _) = {
            let thermal_be = &self.backends.thermal;
            let crash_guard = &self.crash_guard;
            let crash_guard_path = &self.crash_guard_path;
            let hw_applied_recorded = &mut self.hw_applied_recorded;
            step_sensor_loss_cycle(
                &mut self.sensor_loss_guard,
                has_any_temp,
                now_secs,
                self.bios_writes_allowed && self.safe_mode.is_none(),
                self.thermal_guard.is_active(),
                restore_hw_mode,
                restore_cpu_pct,
                restore_gpu_pct,
                cool_hint,
                |m, cpu_pct, gpu_pct, cool| {
                    record_hw_write_attempt(crash_guard, crash_guard_path, hw_applied_recorded);
                    thermal_be.set_dual_fan_mode_with_cool_hint(m, cpu_pct, gpu_pct, cool)
                },
            )
        };
        match sl_dec {
            SensorLossDecision::EngageAutoFallback => {
                self.thermal_guard.reset();
                self.retry_ctrl.reset();
                tracing::warn!(
                    timeout_secs = DEFAULT_SENSOR_LOSS_TIMEOUT_SECS,
                    "All temperature sensors missing; forcing BIOS Auto fan control"
                );
                self.backends.os.send_toast(
                    "Omvix Hub — Sensor Loss",
                    "Temperature sensors lost (>6s). Fans reverted to BIOS Auto mode.",
                    WorkerToastSeverity::Warning,
                );
                if let Ok(mut st) = self.state.lock() {
                    st.sensor_loss_active = true;
                    st.thermal_guard_active = false;
                    st.set_toast(
                        "All thermal sensors lost (>6s) -> BIOS Auto fan fallback",
                        true,
                    );
                }
            }
            SensorLossDecision::RestoreUserMode => {
                tracing::info!(
                    cpu_mode = ?eff_cpu_mode,
                    gpu_mode = ?eff_gpu_mode,
                    "Thermal sensors recovered -> restored user fan control"
                );
                if let Ok(mut st) = self.state.lock() {
                    st.sensor_loss_active = false;
                    st.set_toast("Thermal sensors recovered -> restored fan control", false);
                }
            }
            SensorLossDecision::NoChange => {}
        }

        // Evaluate AC/DC Auto-Profile on transitions (skipped while Safe Mode active)
        if self.safe_mode.is_none() {
            if let Some(ac) = snap.ac_connected {
                if let Some(action) =
                    self.auto_profile_engine
                        .evaluate(&self.cfg.auto_profile, ac, false)
                {
                    tracing::info!(?action, "AC/DC transition triggered auto-profile");
                    self.apply_auto_profile_action(action, ac);
                }
            }
        }

        // Evaluate thermal safety guard & R3 Max-retry at >= 1s intervals
        let mut ran_guard = false;
        let eff_cpu_mode = effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_gpu_mode = effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_guard_mode = combined_fan_guard_mode(eff_cpu_mode, eff_gpu_mode);
        if self.safe_mode.is_none() && !self.sensor_loss_guard.is_active() && guard_due {
            ran_guard = true;
            let guard_cfg = ThermalGuardConfig {
                cpu_threshold_c: self.cfg.thermal_fan.guard_cpu_temp_c,
                sustain_secs: self.cfg.thermal_fan.guard_sustain_secs,
            };
            match self.thermal_guard.evaluate(
                guard_cfg,
                eff_guard_mode,
                snap.cpu_temp_c,
                snap.gpu_temp_c,
                snap.chassis_temp_c,
                snap.cpu_power_w,
                &snap.fan_rpms,
                now_secs,
            ) {
                ThermalGuardDecision::EngageEmergencyMax(reason) => {
                    tracing::warn!(%reason, "Thermal safety guard engaging emergency Max Fan");
                    self.retry_ctrl.begin_engage(&snap.fan_rpms, now_secs);
                    if self.bios_writes_allowed {
                        record_hw_write_attempt(
                            &self.crash_guard,
                            &self.crash_guard_path,
                            &mut self.hw_applied_recorded,
                        );
                        let res = self.backends.thermal.set_fan_mode(FanMode::Max, None);
                        if let Err(ref err) = res {
                            tracing::warn!("Emergency Max Fan write failed: {err}");
                        }
                        match self.retry_ctrl.record_attempt_result(res.is_ok(), now_secs) {
                            GuardMaxAction::FallbackToAuto(msg) => {
                                tracing::error!("{msg}");
                                self.backends.os.send_toast(
                                    "Omvix Hub — Thermal Guard Fallback",
                                    &msg,
                                    WorkerToastSeverity::Error,
                                );
                                if let Err(err) =
                                    self.backends.thermal.set_fan_mode(FanMode::Auto, None)
                                {
                                    tracing::error!("Guard Auto fan fallback failed: {err}");
                                }
                                if let Ok(mut st) = self.state.lock() {
                                    st.thermal_guard_active = true;
                                    st.set_toast(msg, true);
                                }
                            }
                            _ => {
                                self.backends.os.send_toast(
                                    "Omvix Hub — Thermal Guard",
                                    &format!("Emergency Max Fan active ({reason})."),
                                    WorkerToastSeverity::Warning,
                                );
                                if let Ok(mut st) = self.state.lock() {
                                    st.thermal_guard_active = true;
                                    st.set_toast(
                                        format!("Thermal Guard: {reason} -> Max Fan"),
                                        true,
                                    );
                                }
                            }
                        }
                    } else if let Ok(mut st) = self.state.lock() {
                        st.thermal_guard_active = true;
                        st.set_toast(
                            format!("Thermal Guard ({reason}) blocked in read-only board mode"),
                            true,
                        );
                    }
                }
                ThermalGuardDecision::ReleaseEmergency => {
                    tracing::info!("Thermal safety guard releasing emergency Max Fan");
                    self.retry_ctrl.reset();
                    if self.bios_writes_allowed {
                        record_hw_write_attempt(
                            &self.crash_guard,
                            &self.crash_guard_path,
                            &mut self.hw_applied_recorded,
                        );
                        let is_cool = cool_hint_from_snapshot(&snap);
                        let rel_cpu_mode =
                            effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
                        let rel_gpu_mode = effective_gpu_fan_mode(
                            &self.cfg,
                            self.app_profile_engine.active_rule(),
                        );
                        let active_cpu_curve = self
                            .active_curve_override
                            .clone()
                            .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
                        let gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
                        let link_fans = self.cfg.thermal_fan.link_cpu_gpu_fans
                            || self.active_curve_override.is_some();
                        let (hw_mode, cpu_target_pct, gpu_target_pct) =
                            resolve_independent_dual_fan_targets(
                                rel_cpu_mode,
                                rel_gpu_mode,
                                &snap,
                                &active_cpu_curve,
                                &gpu_curve,
                                Some(self.cfg.thermal_fan.manual_speed_pct),
                                Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
                                link_fans,
                                &mut self.prev_curve_temp,
                                &mut self.prev_curve_speed,
                                &mut self.prev_gpu_curve_temp,
                                &mut self.prev_gpu_curve_speed,
                            );
                        if let Err(err) = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                            hw_mode,
                            cpu_target_pct,
                            gpu_target_pct,
                            is_cool,
                        ) {
                            tracing::warn!("Failed releasing Emergency Max Fan: {err}");
                        }
                    }
                    if let Ok(mut st) = self.state.lock() {
                        st.thermal_guard_active = false;
                        st.set_toast("Thermal Guard released", false);
                    }
                }
                ThermalGuardDecision::NoChange => {
                    if self.thermal_guard.is_active() && self.bios_writes_allowed {
                        match self
                            .retry_ctrl
                            .tick(&snap.fan_rpms, &snap.fan_max_rpms, now_secs)
                        {
                            GuardMaxAction::RetryMax { attempt } => {
                                tracing::warn!(attempt, "Retrying Emergency Max Fan command");
                                record_hw_write_attempt(
                                    &self.crash_guard,
                                    &self.crash_guard_path,
                                    &mut self.hw_applied_recorded,
                                );
                                let res = self.backends.thermal.set_fan_mode(FanMode::Max, None);
                                if let Err(ref err) = res {
                                    tracing::warn!(attempt, "Max Fan retry failed: {err}");
                                }
                                if let GuardMaxAction::FallbackToAuto(msg) =
                                    self.retry_ctrl.record_attempt_result(res.is_ok(), now_secs)
                                {
                                    tracing::error!("{msg}");
                                    self.backends.os.send_toast(
                                        "Omvix Hub — Thermal Guard Fallback",
                                        &msg,
                                        WorkerToastSeverity::Error,
                                    );
                                    if let Err(err) =
                                        self.backends.thermal.set_fan_mode(FanMode::Auto, None)
                                    {
                                        tracing::error!("Guard Auto fallback failed: {err}");
                                    }
                                    if let Ok(mut st) = self.state.lock() {
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
                                self.backends.os.send_toast(
                                    "Omvix Hub — Thermal Guard Fallback",
                                    &msg,
                                    WorkerToastSeverity::Error,
                                );
                                record_hw_write_attempt(
                                    &self.crash_guard,
                                    &self.crash_guard_path,
                                    &mut self.hw_applied_recorded,
                                );
                                if let Err(err) =
                                    self.backends.thermal.set_fan_mode(FanMode::Auto, None)
                                {
                                    tracing::error!("Guard Auto fallback failed: {err}");
                                }
                                if let Ok(mut st) = self.state.lock() {
                                    st.set_toast(msg, true);
                                }
                            }
                            GuardMaxAction::Wait => {}
                        }
                    }
                }
            }
        }

        // Evaluate custom / mixed fan curves (every 2s) when emergency & sensor-loss guards are inactive
        let mut ran_curve = false;
        let eff_cpu_mode = effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_gpu_mode = effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let has_dynamic_fan_channel = !(eff_cpu_mode == FanMode::Auto
            && eff_gpu_mode == FanMode::Auto)
            && !(eff_cpu_mode == FanMode::Max && eff_gpu_mode == FanMode::Max)
            && (eff_cpu_mode == FanMode::CustomCurve
                || eff_gpu_mode == FanMode::CustomCurve
                || eff_cpu_mode == FanMode::Auto
                || eff_gpu_mode == FanMode::Auto);
        if self.safe_mode.is_none()
            && self.bios_writes_allowed
            && !self.sensor_loss_guard.is_active()
            && !self.thermal_guard.is_active()
            && has_dynamic_fan_channel
            && curve_due
        {
            ran_curve = true;
            let link_fans =
                self.cfg.thermal_fan.link_cpu_gpu_fans || self.active_curve_override.is_some();
            let (cpu_temp_opt, cpu_target) = match eff_cpu_mode {
                FanMode::Max => (None, 100),
                FanMode::Manual => (None, self.cfg.thermal_fan.manual_speed_pct.clamp(0, 100)),
                FanMode::CustomCurve => {
                    if let Some(ct) = cpu_curve_temp_c(&snap, link_fans) {
                        let active_cpu_curve = self
                            .active_curve_override
                            .as_ref()
                            .unwrap_or(&self.cfg.thermal_fan.custom_curve);
                        (
                            Some(ct),
                            active_cpu_curve.evaluate(
                                ct,
                                self.prev_curve_temp,
                                self.prev_curve_speed,
                            ),
                        )
                    } else {
                        (None, self.prev_curve_speed)
                    }
                }
                FanMode::Auto => {
                    if let Some(ct) = cpu_curve_temp_c(&snap, false) {
                        (
                            Some(ct),
                            FanCurve::preset_ogh_like().evaluate(
                                ct,
                                self.prev_curve_temp,
                                self.prev_curve_speed,
                            ),
                        )
                    } else {
                        (None, self.prev_curve_speed)
                    }
                }
            };
            let (gpu_temp_opt, gpu_target) = if link_fans {
                (None, cpu_target)
            } else {
                match eff_gpu_mode {
                    FanMode::Max => (None, 100),
                    FanMode::Manual => (
                        None,
                        self.cfg.thermal_fan.gpu_manual_speed_pct.clamp(0, 100),
                    ),
                    FanMode::CustomCurve => {
                        if let Some(gt) = gpu_curve_temp_c(&snap) {
                            (
                                Some(gt),
                                self.cfg.thermal_fan.gpu_custom_curve.evaluate(
                                    gt,
                                    self.prev_gpu_curve_temp,
                                    self.prev_gpu_curve_speed,
                                ),
                            )
                        } else {
                            (None, self.prev_gpu_curve_speed)
                        }
                    }
                    FanMode::Auto => {
                        if let Some(gt) = gpu_curve_temp_c(&snap) {
                            (
                                Some(gt),
                                FanCurve::preset_ogh_like().evaluate(
                                    gt,
                                    self.prev_gpu_curve_temp,
                                    self.prev_gpu_curve_speed,
                                ),
                            )
                        } else {
                            (None, self.prev_gpu_curve_speed)
                        }
                    }
                }
            };

            let cpu_changed = cpu_target != self.prev_curve_speed;
            let gpu_changed = gpu_target != self.prev_gpu_curve_speed;
            let is_cool = cool_hint_from_snapshot(&snap);
            let cool_edge = self.prev_curve_cool_hint != Some(is_cool)
                && (cpu_target <= 30 || gpu_target <= 30);
            if !self.prev_curve_applied || cpu_changed || gpu_changed || cool_edge {
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                if let Err(err) = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                    FanMode::CustomCurve,
                    Some(cpu_target),
                    Some(gpu_target),
                    is_cool,
                ) {
                    tracing::warn!(
                        cpu_target,
                        gpu_target,
                        "Custom fan curve write failed: {err}"
                    );
                } else {
                    self.prev_curve_applied = true;
                    self.prev_curve_cool_hint = Some(is_cool);
                    self.prev_curve_speed = cpu_target;
                    if let Some(ct) = cpu_temp_opt {
                        if cpu_changed || ct > self.prev_curve_temp {
                            self.prev_curve_temp = ct;
                        }
                    }
                    self.prev_gpu_curve_speed = gpu_target;
                    if let Some(gt) = gpu_temp_opt {
                        if gpu_changed || gt > self.prev_gpu_curve_temp {
                            self.prev_gpu_curve_temp = gt;
                        }
                    }
                }
            } else {
                if let Some(ct) = cpu_temp_opt {
                    if ct > self.prev_curve_temp {
                        // Track rising peaks without shifting baseline down while hysteresis holds speed
                        self.prev_curve_temp = ct;
                    }
                }
                if let Some(gt) = gpu_temp_opt {
                    if gt > self.prev_gpu_curve_temp {
                        self.prev_gpu_curve_temp = gt;
                    }
                }
            }
        }

        (ran_guard, ran_curve)
    }

    fn poll_app_profiles_step(&mut self) -> bool {
        let running = if self.cfg.app_profiles.enabled {
            self.backends.os.running_process_names_lower()
        } else {
            std::collections::HashSet::new()
        };
        let state = &self.state;
        let cfg = &self.cfg;
        let active_curve_override = &self.active_curve_override;
        let transition = self
            .app_profile_engine
            .evaluate(&cfg.app_profiles, &running, || {
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
                    gpu_fan_mode: cfg.thermal_fan.gpu_fan_mode,
                    manual_speed_pct: cfg.thermal_fan.manual_speed_pct,
                    gpu_manual_speed_pct: cfg.thermal_fan.gpu_manual_speed_pct,
                    custom_curve: active_curve_override
                        .clone()
                        .unwrap_or_else(|| cfg.thermal_fan.custom_curve.clone()),
                    gpu_custom_curve: cfg.thermal_fan.gpu_custom_curve.clone(),
                    link_cpu_gpu_fans: cfg.thermal_fan.link_cpu_gpu_fans,
                    refresh_hz,
                    cpu_boost_mode,
                }
            });
        let discrete_transition = matches!(
            transition,
            AppProfileTransition::Activate { .. } | AppProfileTransition::Restore(_)
        );
        self.apply_app_profile_transition(transition);
        discrete_transition
    }

    fn poll_battery_step(&mut self) -> bool {
        let Ok(bat) = self.backends.battery.get_battery_status() else {
            return false;
        };
        let conflict_active = self
            .state
            .lock()
            .map(|s| s.hp_charge_service_running)
            .unwrap_or(false);
        let own_svc_running = self.backends.os.is_battery_service_running();
        let mut travel_state_synced = false;
        if own_svc_running {
            if let Some(svc_bat) = self.backends.os.read_battery_service_config() {
                if (
                    svc_bat.one_time_full_charge,
                    svc_bat.travel_charge_until_utc,
                ) != (
                    self.cfg.battery.one_time_full_charge,
                    self.cfg.battery.travel_charge_until_utc,
                ) {
                    self.cfg.battery.one_time_full_charge = svc_bat.one_time_full_charge;
                    self.cfg.battery.travel_charge_until_utc = svc_bat.travel_charge_until_utc;
                    self.last_committed_battery.one_time_full_charge = svc_bat.one_time_full_charge;
                    self.last_committed_battery.travel_charge_until_utc =
                        svc_bat.travel_charge_until_utc;
                    travel_state_synced = true;
                }
            }
        }
        if self.safe_mode.is_none()
            && self.bios_writes_allowed
            && self.cfg.battery.enabled
            && !conflict_active
            && !own_svc_running
        {
            let prev_travel = (
                self.cfg.battery.one_time_full_charge,
                self.cfg.battery.travel_charge_until_utc,
            );
            let target = self.battery_engine.evaluate(
                &mut self.cfg.battery,
                bat.charge_percent,
                bat.ac_connected,
            );
            if (
                self.cfg.battery.one_time_full_charge,
                self.cfg.battery.travel_charge_until_utc,
            ) != prev_travel
            {
                self.last_committed_battery.one_time_full_charge =
                    self.cfg.battery.one_time_full_charge;
                self.last_committed_battery.travel_charge_until_utc =
                    self.cfg.battery.travel_charge_until_utc;
                let _ = self
                    .backends
                    .os
                    .sync_battery_service_config(&self.cfg.battery);
                travel_state_synced = true;
            }
            if self.last_applied_bat_mode != Some(target) {
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                if let Err(err) = self.backends.battery.set_charge_mode(target) {
                    tracing::debug!("Auto battery limit skipped: {err}");
                } else {
                    self.last_applied_bat_mode = Some(target);
                }
            }
        }
        let travel_active = self
            .cfg
            .battery
            .is_travel_charge_active(chrono::Utc::now().timestamp());
        if self.cfg.battery.enabled
            && !travel_active
            && bat.ac_connected
            && bat.charge_percent >= self.cfg.battery.stop_threshold_pct
        {
            if !self.bat_limit_toast_sent {
                self.bat_limit_toast_sent = true;
                self.backends.os.send_toast(
                    "Omvix Hub — Battery Care",
                    &format!(
                        "Battery charge limit reached ({}%). Charging paused.",
                        bat.charge_percent
                    ),
                    WorkerToastSeverity::Info,
                );
            }
        } else if !self.cfg.battery.enabled
            || !bat.ac_connected
            || bat.charge_percent < self.cfg.battery.start_threshold_pct
        {
            self.bat_limit_toast_sent = false;
        }
        if let Ok(mut st) = self.state.lock() {
            st.battery_status = Some(bat);
            if travel_state_synced {
                st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                st.synced_config = Some(self.cfg.clone());
            }
        }
        true
    }

    fn poll_keep_alive_step(&mut self) -> bool {
        if !should_run_keep_alive(
            self.cfg.thermal_fan.keep_alive_enabled,
            self.sensor_loss_guard.is_active(),
            self.bios_writes_allowed && self.safe_mode.is_none(),
        ) {
            return false;
        }
        record_hw_write_attempt(
            &self.crash_guard,
            &self.crash_guard_path,
            &mut self.hw_applied_recorded,
        );
        let is_cool = current_cool_hint(&self.state);
        if let Err(err) = self
            .backends
            .thermal
            .keep_alive_tick_with_cool_hint(is_cool)
        {
            tracing::warn!("BIOS fan keep-alive tick failed: {err}");
        }
        true
    }

    fn handle_command(
        &mut self,
        cmd: WorkerCommand,
        self_tx: &Sender<WorkerCommand>,
        ctx: &Option<egui::Context>,
        window_visible: &Arc<AtomicBool>,
    ) -> bool {
        match cmd {
            WorkerCommand::PollExternalHardwareState => {
                if self.poll_external_hw_state_step(false) {
                    request_repaint_always(ctx);
                }
            }
            WorkerCommand::Shutdown => {
                tracing::info!("Background worker shutting down");
                self.apply_off_state_safety("clean shutdown");
                if let Ok(mut cg) = self.crash_guard.lock() {
                    cg.mark_clean_exit(current_unix_secs());
                    let _ = cg.save_to_path(&self.crash_guard_path);
                }
                return true;
            }
            WorkerCommand::ExitSafeMode => {
                self.safe_mode = None;
                if let Ok(mut st) = self.state.lock() {
                    st.safe_mode = None;
                    st.set_toast("Exited Safe Mode — hardware controls enabled", false);
                }
                tracing::info!("User exited Safe Mode; hardware controls enabled");
            }
            WorkerCommand::SystemSuspending(ack) => {
                tracing::info!("System suspending -> applying Fan Auto & Battery off-state safety");
                self.sync_keyboard_backlight_from_hw(false);
                self.apply_off_state_safety("system suspend");
                let _ = ack.send(());
            }
            WorkerCommand::SystemShuttingDown(ack) => {
                tracing::info!(
                    "System shutting down -> applying Fan Auto & Battery off-state safety"
                );
                self.apply_off_state_safety("system shutdown");
                if let Ok(mut cg) = self.crash_guard.lock() {
                    cg.mark_clean_exit(current_unix_secs());
                    let _ = cg.save_to_path(&self.crash_guard_path);
                }
                let _ = ack.send(());
            }
            WorkerCommand::SystemResumed => {
                tracing::info!(
                    "System resumed from sleep/standby -> restoring Omvix hardware state"
                );
                self.thermal_guard.reset();
                self.sensor_loss_guard.reset();
                self.retry_ctrl.reset();
                self.last_applied_bat_mode = None;
                let eff_perf = self
                    .app_profile_engine
                    .active_rule()
                    .map(|r| r.performance_mode)
                    .unwrap_or(self.cfg.thermal_fan.performance_mode);
                let eff_cpu_fan =
                    effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
                let eff_gpu_fan =
                    effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
                if self.bios_writes_allowed && self.safe_mode.is_none() {
                    record_hw_write_attempt(
                        &self.crash_guard,
                        &self.crash_guard_path,
                        &mut self.hw_applied_recorded,
                    );
                    if let Err(err) = self.backends.thermal.set_performance_mode(eff_perf) {
                        tracing::warn!("Failed restoring performance mode on resume: {err}");
                    }
                    self.sync_win_power_mode_if_enabled(eff_perf);

                    let snap = self
                        .state
                        .lock()
                        .map(|st| st.telemetry.clone())
                        .unwrap_or_default();
                    let is_cool = cool_hint_from_snapshot(&snap);
                    let active_cpu_curve = self
                        .active_curve_override
                        .clone()
                        .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
                    let gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
                    let link_fans = self.cfg.thermal_fan.link_cpu_gpu_fans
                        || self.active_curve_override.is_some();
                    let (hw_mode, cpu_target_pct, gpu_target_pct) =
                        resolve_independent_dual_fan_targets(
                            eff_cpu_fan,
                            eff_gpu_fan,
                            &snap,
                            &active_cpu_curve,
                            &gpu_curve,
                            Some(self.cfg.thermal_fan.manual_speed_pct),
                            Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
                            link_fans,
                            &mut self.prev_curve_temp,
                            &mut self.prev_curve_speed,
                            &mut self.prev_gpu_curve_temp,
                            &mut self.prev_gpu_curve_speed,
                        );
                    if let Err(err) = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                        hw_mode,
                        cpu_target_pct,
                        gpu_target_pct,
                        is_cool,
                    ) {
                        tracing::warn!("Failed restoring fan mode on resume: {err}");
                    }
                    if self.cfg.lighting.enabled && !self.keyboard_dimmed_by_auto_profile {
                        match self.backends.kbd.set_backlight(
                            true,
                            self.cfg.lighting.brightness_pct,
                            &self.cfg.lighting.zones,
                        ) {
                            Ok(()) => {
                                self.last_kbd_write_at = Some(Instant::now());
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "Failed restoring keyboard backlight on resume: {err}"
                                );
                            }
                        }
                    }
                } else if self.safe_mode.is_some() {
                    tracing::info!(
                        "System resumed while Safe Mode active; skipping hardware restore"
                    );
                }
                if let Ok(mut st) = self.state.lock() {
                    st.thermal_guard_active = false;
                    st.sensor_loss_active = false;
                }
                self.poll_external_hw_state_step(false);
                tracing::info!("Re-applied thermal, fan, and lighting state after system resume");
            }
            WorkerCommand::UpdateConfig(mut new_cfg) => {
                let _ = new_cfg.thermal_fan.custom_curve.normalize();
                let _ = new_cfg.thermal_fan.gpu_custom_curve.normalize();
                let cpu_curve_changed = self.cfg.thermal_fan.custom_curve.points
                    != new_cfg.thermal_fan.custom_curve.points;
                let gpu_curve_changed = self.cfg.thermal_fan.gpu_custom_curve.points
                    != new_cfg.thermal_fan.gpu_custom_curve.points;
                let link_changed =
                    self.cfg.thermal_fan.link_cpu_gpu_fans != new_cfg.thermal_fan.link_cpu_gpu_fans;
                if cpu_curve_changed {
                    self.active_curve_override = None;
                }
                let curve_changed = cpu_curve_changed || gpu_curve_changed || link_changed;
                let bat_changed = self.cfg.battery.enabled != new_cfg.battery.enabled
                    || self.cfg.battery.mode != new_cfg.battery.mode
                    || self.cfg.battery.stop_threshold_pct != new_cfg.battery.stop_threshold_pct
                    || self.cfg.battery.start_threshold_pct != new_cfg.battery.start_threshold_pct
                    || self.cfg.battery.hold_when_shutdown != new_cfg.battery.hold_when_shutdown
                    || self.cfg.battery.one_time_full_charge
                        != new_cfg.battery.one_time_full_charge
                    || self.cfg.battery.travel_charge_until_utc
                        != new_cfg.battery.travel_charge_until_utc;
                let was_allowed = self.bios_writes_allowed;
                self.cfg = new_cfg;
                self.bios_writes_allowed = is_bios_write_allowed(
                    &self.board_id,
                    self.cfg.general.experimental_board_id.as_deref(),
                );
                if !was_allowed && self.bios_writes_allowed {
                    tracing::info!(board_id = %self.board_id, "Experimental BIOS write mode approved by user");
                }
                if let Ok(mut st) = self.state.lock() {
                    st.bios_writes_allowed = self.bios_writes_allowed;
                }
                if bat_changed && self.safe_mode.is_none() {
                    if let Err(err) = self
                        .backends
                        .os
                        .sync_battery_service_config(&self.cfg.battery)
                    {
                        tracing::debug!("Failed syncing battery service config: {err}");
                    }
                }
                let eff_cpu_fan =
                    effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
                let eff_gpu_fan =
                    effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
                if let Ok(mut st) = self.state.lock() {
                    st.effective_gpu_fan_mode = eff_gpu_fan;
                }
                let should_apply_fan = curve_changed
                    && (eff_cpu_fan == FanMode::CustomCurve
                        || eff_gpu_fan == FanMode::CustomCurve
                        || (link_changed
                            && (eff_cpu_fan != FanMode::Auto || eff_gpu_fan != FanMode::Auto)));
                if self.safe_mode.is_none()
                    && self.bios_writes_allowed
                    && should_apply_fan
                    && !self.thermal_guard.is_active()
                    && !self.sensor_loss_guard.is_active()
                {
                    let snap = self
                        .state
                        .lock()
                        .map(|st| st.telemetry.clone())
                        .unwrap_or_default();
                    let is_cool = cool_hint_from_snapshot(&snap);
                    let active_cpu_curve = self
                        .active_curve_override
                        .clone()
                        .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
                    let gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
                    let link_fans = self.cfg.thermal_fan.link_cpu_gpu_fans
                        || self.active_curve_override.is_some();
                    let (hw_mode, cpu_target_pct, gpu_target_pct) =
                        resolve_independent_dual_fan_targets(
                            eff_cpu_fan,
                            eff_gpu_fan,
                            &snap,
                            &active_cpu_curve,
                            &gpu_curve,
                            Some(self.cfg.thermal_fan.manual_speed_pct),
                            Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
                            link_fans,
                            &mut self.prev_curve_temp,
                            &mut self.prev_curve_speed,
                            &mut self.prev_gpu_curve_temp,
                            &mut self.prev_gpu_curve_speed,
                        );
                    record_hw_write_attempt(
                        &self.crash_guard,
                        &self.crash_guard_path,
                        &mut self.hw_applied_recorded,
                    );
                    if let Err(err) = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                        hw_mode,
                        cpu_target_pct,
                        gpu_target_pct,
                        is_cool,
                    ) {
                        self.prev_curve_applied = false;
                        tracing::warn!("Failed applying updated fan settings: {err}");
                    } else {
                        self.prev_curve_applied = true;
                        self.prev_curve_cool_hint = Some(is_cool);
                    }
                }
            }
            WorkerCommand::SetTelemetryIntervalMs(ms) => {
                self.cfg.general.telemetry_interval_ms = ms.clamp(100, 10_000);
            }
            WorkerCommand::SetTelemetryPaused(paused) => {
                if let Ok(mut st) = self.state.lock() {
                    st.telemetry_paused = paused;
                }
            }
            WorkerCommand::ResetTelemetryStats => {
                if let Ok(mut st) = self.state.lock() {
                    st.clear_telemetry_history();
                }
            }
            WorkerCommand::SetPerformanceMode(mode) => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "SetPerformanceMode",
                ) {
                    return false;
                }
                let prev_mode = self.cfg.thermal_fan.performance_mode;
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                self.cfg.thermal_fan.performance_mode = mode;
                let res = self.backends.thermal.set_performance_mode(mode);
                if res.is_ok() {
                    self.sync_win_power_mode_if_enabled(mode);
                    if let Ok(mut st) = self.state.lock() {
                        st.effective_performance_mode = mode;
                    }
                    record_history_entry(
                        &mut self.change_history,
                        &self.state,
                        ChangeDomain::PerformanceMode,
                        format!("{prev_mode:?}"),
                        format!("{mode:?}"),
                        RevertPayload::PerformanceMode(prev_mode),
                    );
                }
                record_result(&self.state, res, format!("Performance mode -> {mode:?}"));
            }
            WorkerCommand::SetFanMode {
                mode,
                gpu_mode,
                manual_speed_pct,
                gpu_manual_speed_pct,
            } => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "SetFanMode",
                ) {
                    return false;
                }
                let prev_mode = self.cfg.thermal_fan.fan_mode;
                let prev_gpu_mode = self.cfg.thermal_fan.gpu_fan_mode;
                let prev_pct = self.cfg.thermal_fan.manual_speed_pct;
                let prev_gpu_pct = self.cfg.thermal_fan.gpu_manual_speed_pct;
                let prev_link = self.cfg.thermal_fan.link_cpu_gpu_fans;
                let prev_curve = self
                    .active_curve_override
                    .clone()
                    .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
                let prev_gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
                let old_sum = format_fan_summary(prev_mode, prev_gpu_mode, prev_pct, prev_gpu_pct);

                let resolved_gpu_mode = if self.cfg.thermal_fan.link_cpu_gpu_fans {
                    mode
                } else {
                    gpu_mode.unwrap_or(self.cfg.thermal_fan.gpu_fan_mode)
                };

                if self.sensor_loss_guard.is_active()
                    && (mode != FanMode::Auto || resolved_gpu_mode != FanMode::Auto)
                {
                    if let Ok(mut st) = self.state.lock() {
                        st.set_toast(
                            "Thermal sensors lost: Auto fallback active until sensors recover",
                            true,
                        );
                    }
                    return false;
                }
                if self.thermal_guard.is_active()
                    && (mode != FanMode::Max || resolved_gpu_mode != FanMode::Max)
                {
                    if let Ok(mut st) = self.state.lock() {
                        st.set_toast(
                            "Thermal Guard active: Max Fan remains engaged until temperatures drop",
                            true,
                        );
                    }
                    return false;
                }
                let next_cpu_pct = manual_speed_pct
                    .map(|p| p.clamp(0, 100))
                    .unwrap_or(self.cfg.thermal_fan.manual_speed_pct);
                let next_gpu_pct = gpu_manual_speed_pct
                    .map(|p| p.clamp(0, 100))
                    .unwrap_or(self.cfg.thermal_fan.gpu_manual_speed_pct);
                let snap = self
                    .state
                    .lock()
                    .map(|st| st.telemetry.clone())
                    .unwrap_or_default();
                let is_cool = cool_hint_from_snapshot(&snap);
                let link_fans = self.cfg.thermal_fan.link_cpu_gpu_fans;
                let mut next_prev_temp = self.prev_curve_temp;
                let mut next_prev_speed = self.prev_curve_speed;
                let mut next_prev_gpu_temp = self.prev_gpu_curve_temp;
                let mut next_prev_gpu_speed = self.prev_gpu_curve_speed;
                let (hw_mode, cpu_target_pct, gpu_target_pct) =
                    resolve_independent_dual_fan_targets(
                        mode,
                        resolved_gpu_mode,
                        &snap,
                        &self.cfg.thermal_fan.custom_curve,
                        &self.cfg.thermal_fan.gpu_custom_curve,
                        Some(next_cpu_pct),
                        Some(next_gpu_pct),
                        link_fans,
                        &mut next_prev_temp,
                        &mut next_prev_speed,
                        &mut next_prev_gpu_temp,
                        &mut next_prev_gpu_speed,
                    );
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                    hw_mode,
                    cpu_target_pct,
                    gpu_target_pct,
                    is_cool,
                );
                if res.is_ok() {
                    self.active_curve_override = None;
                    self.cfg.thermal_fan.fan_mode = mode;
                    self.cfg.thermal_fan.gpu_fan_mode = resolved_gpu_mode;
                    self.cfg.thermal_fan.manual_speed_pct = next_cpu_pct;
                    self.cfg.thermal_fan.gpu_manual_speed_pct = next_gpu_pct;
                    self.retry_ctrl.reset();
                    self.prev_curve_temp = next_prev_temp;
                    self.prev_curve_speed = next_prev_speed;
                    self.prev_gpu_curve_temp = next_prev_gpu_temp;
                    self.prev_gpu_curve_speed = next_prev_gpu_speed;
                    self.prev_curve_applied = true;
                    self.prev_curve_cool_hint = Some(is_cool);
                    if let Ok(mut st) = self.state.lock() {
                        st.effective_fan_mode = mode;
                        st.effective_gpu_fan_mode = resolved_gpu_mode;
                    }
                    let new_sum = format_fan_summary(
                        mode,
                        resolved_gpu_mode,
                        self.cfg.thermal_fan.manual_speed_pct,
                        if link_fans {
                            self.cfg.thermal_fan.manual_speed_pct
                        } else {
                            self.cfg.thermal_fan.gpu_manual_speed_pct
                        },
                    );
                    record_history_entry(
                        &mut self.change_history,
                        &self.state,
                        ChangeDomain::FanMode,
                        old_sum,
                        new_sum.clone(),
                        RevertPayload::FanMode {
                            mode: prev_mode,
                            gpu_mode: prev_gpu_mode,
                            manual_speed_pct: prev_pct,
                            gpu_manual_speed_pct: prev_gpu_pct,
                            custom_curve: prev_curve,
                            gpu_custom_curve: prev_gpu_curve,
                            link_cpu_gpu_fans: prev_link,
                        },
                    );
                    record_result(&self.state, res, format!("Fan -> {new_sum}"));
                } else {
                    record_result(&self.state, res, format!("Fan mode -> {mode:?}"));
                }
            }
            WorkerCommand::ApplyBatteryNow => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "Battery 0x2B",
                ) {
                    return false;
                }
                let _ = self
                    .backends
                    .os
                    .sync_battery_service_config(&self.cfg.battery);
                if let Ok(bat) = self.backends.battery.get_battery_status() {
                    let prev_bat = self.last_committed_battery.clone();
                    let prev_travel = (
                        self.cfg.battery.one_time_full_charge,
                        self.cfg.battery.travel_charge_until_utc,
                    );
                    let target = self.battery_engine.evaluate(
                        &mut self.cfg.battery,
                        bat.charge_percent,
                        bat.ac_connected,
                    );
                    if (
                        self.cfg.battery.one_time_full_charge,
                        self.cfg.battery.travel_charge_until_utc,
                    ) != prev_travel
                    {
                        let _ = self
                            .backends
                            .os
                            .sync_battery_service_config(&self.cfg.battery);
                        if let Ok(mut st) = self.state.lock() {
                            st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                            st.synced_config = Some(self.cfg.clone());
                        }
                    }
                    if should_skip_battery_apply(
                        bat.current_mode,
                        self.last_applied_bat_mode,
                        target,
                    ) {
                        record_history_entry(
                            &mut self.change_history,
                            &self.state,
                            ChangeDomain::BatteryLimit,
                            format_battery_summary(&prev_bat),
                            format_battery_summary(&self.cfg.battery),
                            RevertPayload::BatteryLimit {
                                enabled: prev_bat.enabled,
                                mode: prev_bat.mode,
                                stop_threshold_pct: prev_bat.stop_threshold_pct,
                                start_threshold_pct: prev_bat.start_threshold_pct,
                                hold_when_shutdown: prev_bat.hold_when_shutdown,
                                one_time_full_charge: prev_bat.one_time_full_charge,
                                travel_charge_until_utc: prev_bat.travel_charge_until_utc,
                                active_profile_id: prev_bat.active_profile_id,
                            },
                        );
                        self.last_committed_battery = self.cfg.battery.clone();
                        if let Ok(mut st) = self.state.lock() {
                            st.battery_status = Some(bat);
                            st.set_toast(format!("Battery charge mode -> {target:?}"), false);
                        }
                    } else {
                        record_hw_write_attempt(
                            &self.crash_guard,
                            &self.crash_guard_path,
                            &mut self.hw_applied_recorded,
                        );
                        let res = self.backends.battery.set_charge_mode(target);
                        if res.is_ok() {
                            self.last_applied_bat_mode = Some(target);
                            record_history_entry(
                                &mut self.change_history,
                                &self.state,
                                ChangeDomain::BatteryLimit,
                                format_battery_summary(&prev_bat),
                                format_battery_summary(&self.cfg.battery),
                                RevertPayload::BatteryLimit {
                                    enabled: prev_bat.enabled,
                                    mode: prev_bat.mode,
                                    stop_threshold_pct: prev_bat.stop_threshold_pct,
                                    start_threshold_pct: prev_bat.start_threshold_pct,
                                    hold_when_shutdown: prev_bat.hold_when_shutdown,
                                    one_time_full_charge: prev_bat.one_time_full_charge,
                                    travel_charge_until_utc: prev_bat.travel_charge_until_utc,
                                    active_profile_id: prev_bat.active_profile_id,
                                },
                            );
                            self.last_committed_battery = self.cfg.battery.clone();
                        }
                        if let Ok(refreshed) = self.backends.battery.get_battery_status() {
                            if let Ok(mut st) = self.state.lock() {
                                st.battery_status = Some(refreshed);
                            }
                        }
                        record_result(
                            &self.state,
                            res,
                            format!("Battery charge mode -> {target:?}"),
                        );
                    }
                }
            }
            WorkerCommand::ApplyLighting => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "Keyboard Lighting",
                ) {
                    return false;
                }
                let prev_light = self.last_committed_lighting.clone();
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.kbd.set_backlight(
                    self.cfg.lighting.enabled,
                    self.cfg.lighting.brightness_pct,
                    &self.cfg.lighting.zones,
                );
                if res.is_ok() {
                    self.last_kbd_write_at = Some(Instant::now());
                    self.keyboard_dimmed_by_auto_profile = false;
                    record_history_entry(
                        &mut self.change_history,
                        &self.state,
                        ChangeDomain::KeyboardLighting,
                        format_lighting_summary(&prev_light),
                        format_lighting_summary(&self.cfg.lighting),
                        RevertPayload::KeyboardLighting {
                            enabled: prev_light.enabled,
                            brightness_pct: prev_light.brightness_pct,
                            zones: prev_light.zones,
                        },
                    );
                    self.last_committed_lighting = self.cfg.lighting.clone();
                }
                record_result(&self.state, res, "Keyboard lighting updated".to_string());
            }
            WorkerCommand::SetRefreshRate(hz) => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    false,
                    &self.state,
                    "SetRefreshRate",
                ) {
                    return false;
                }
                let prev_hz = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.display_mode.as_ref().map(|d| d.current_hz));
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.display.set_refresh_rate(hz);
                if res.is_ok() {
                    if let Some(old_hz) = prev_hz {
                        record_history_entry(
                            &mut self.change_history,
                            &self.state,
                            ChangeDomain::DisplayRefreshRate,
                            format!("{old_hz} Hz"),
                            format!("{hz} Hz"),
                            RevertPayload::DisplayRefreshRate(old_hz),
                        );
                    }
                    if let Ok(disp) = self.backends.display.get_display_mode() {
                        if let Ok(mut st) = self.state.lock() {
                            st.display_mode = Some(disp);
                        }
                    }
                }
                record_result(&self.state, res, format!("Refresh rate -> {hz} Hz"));
            }
            WorkerCommand::SetGpuMode(mode) => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "GPU MUX Switch",
                ) {
                    return false;
                }
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.gpu.set_gpu_mode(mode);
                if res.is_ok() {
                    if let Ok(mut st) = self.state.lock() {
                        st.gpu_mode = Some(mode);
                        st.gpu_mux_reboot_pending = true;
                    }
                }
                record_result(
                    &self.state,
                    res,
                    format!("GPU MUX -> {mode:?} (reboot required)"),
                );
            }
            WorkerCommand::SetCpuBoost(mode) => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    false,
                    &self.state,
                    "SetCpuBoost",
                ) {
                    return false;
                }
                let prev_boost = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.cpu_boost)
                    .or(self.cfg.tuning.cpu_boost_mode);
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.tuning.set_cpu_boost_mode(mode);
                if res.is_ok() {
                    self.cfg.tuning.cpu_boost_mode = Some(mode);
                    if let Ok(mut st) = self.state.lock() {
                        st.cpu_boost = Some(mode);
                    }
                    if let Some(old_boost) = prev_boost {
                        record_history_entry(
                            &mut self.change_history,
                            &self.state,
                            ChangeDomain::CpuBoost,
                            format!("{old_boost:?}"),
                            format!("{mode:?}"),
                            RevertPayload::CpuBoost(old_boost),
                        );
                    }
                }
                record_result(&self.state, res, format!("CPU Boost -> {mode:?}"));
            }
            WorkerCommand::SetPowerLimits { pl1, pl2 } => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "Power Limits",
                ) {
                    return false;
                }
                let prev_limits = self.last_committed_power_limits;
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let res = self.backends.tuning.set_power_limits(pl1, pl2);
                if res.is_ok() {
                    self.cfg.tuning.pl1_watts = pl1;
                    self.cfg.tuning.pl2_watts = pl2;
                    self.last_committed_power_limits = (pl1, pl2);
                    if prev_limits != (pl1, pl2) {
                        record_history_entry(
                            &mut self.change_history,
                            &self.state,
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
                record_result(
                    &self.state,
                    res,
                    format!("Power limits PL1={pl1:?} PL2={pl2:?}"),
                );
            }
            WorkerCommand::RestoreDefaultPowerLimits { keep_pl1, keep_pl2 } => {
                if !check_hw_write_allowed(
                    self.safe_mode,
                    self.bios_writes_allowed,
                    true,
                    &self.state,
                    "Power Limits",
                ) {
                    return false;
                }
                let prev_limits = self.last_committed_power_limits;
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let mut res = self.restore_ec_default_power_limits_via_profile();
                if res.is_ok() && (keep_pl1.is_some() || keep_pl2.is_some()) {
                    res = self.backends.tuning.set_power_limits(keep_pl1, keep_pl2);
                }
                if res.is_ok() {
                    self.cfg.tuning.pl1_watts = keep_pl1;
                    self.cfg.tuning.pl2_watts = keep_pl2;
                    self.last_committed_power_limits = (keep_pl1, keep_pl2);
                    if prev_limits != (keep_pl1, keep_pl2) {
                        record_history_entry(
                            &mut self.change_history,
                            &self.state,
                            ChangeDomain::PowerLimits,
                            format_power_limits_summary(prev_limits.0, prev_limits.1),
                            format_power_limits_summary(keep_pl1, keep_pl2),
                            RevertPayload::PowerLimits {
                                pl1_watts: prev_limits.0,
                                pl2_watts: prev_limits.1,
                            },
                        );
                    }
                }
                let ok_msg = if keep_pl1.is_none() && keep_pl2.is_none() {
                    "Power limits restored to BIOS/EC defaults".to_string()
                } else {
                    format!(
                        "Power limits PL1={keep_pl1:?} PL2={keep_pl2:?} (disabled channel reset to BIOS default)"
                    )
                };
                record_result(&self.state, res, ok_msg);
            }
            WorkerCommand::InstallPawnIo => {
                let already_running = self
                    .state
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
                    let state_bg = Arc::clone(&self.state);
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
                let active = self.backends.telemetry.reload_pawnio();
                let sec = system_info::collect_security_status();
                if let Ok(mut st) = self.state.lock() {
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
                let bat_cfg = self.cfg.battery.clone();
                spawn_battery_service_task(
                    &self.state,
                    self_tx,
                    ctx,
                    window_visible,
                    "Installing / updating OmvixBatteryService...",
                    move || omvix_windows::battery_service::install_or_update_service(&bat_cfg),
                );
            }
            WorkerCommand::StartBatteryService => {
                spawn_battery_service_task(
                    &self.state,
                    self_tx,
                    ctx,
                    window_visible,
                    "Starting OmvixBatteryService...",
                    omvix_windows::battery_service::start_service,
                );
            }
            WorkerCommand::StopBatteryService => {
                spawn_battery_service_task(
                    &self.state,
                    self_tx,
                    ctx,
                    window_visible,
                    "Stopping OmvixBatteryService...",
                    omvix_windows::battery_service::stop_service,
                );
            }
            WorkerCommand::RestartBatteryService => {
                spawn_battery_service_task(
                    &self.state,
                    self_tx,
                    ctx,
                    window_visible,
                    "Restarting OmvixBatteryService...",
                    omvix_windows::battery_service::restart_service,
                );
            }
            WorkerCommand::UninstallBatteryServiceZeroTrace => {
                spawn_battery_service_task(
                    &self.state,
                    self_tx,
                    ctx,
                    window_visible,
                    "Removing OmvixBatteryService & cleaning all traces...",
                    omvix_windows::battery_service::uninstall_service_zero_trace,
                );
            }
            WorkerCommand::RefreshBatteryService => {
                self.last_applied_bat_mode = None;
                let svc_info = omvix_windows::battery_service::query_service_info();
                let copy_status = omvix_windows::autostart::query_installed_copy_status();
                let bat = self.backends.battery.get_battery_status().ok();
                if let Ok(mut st) = self.state.lock() {
                    st.battery_service = svc_info;
                    st.installed_copy_status = copy_status;
                    if bat.is_some() {
                        st.battery_status = bat;
                    }
                }
            }
            WorkerCommand::SetAutostart(enabled) => {
                self.cfg.general.start_with_windows = enabled;
                let res = std::env::current_exe()
                    .map_err(Into::into)
                    .and_then(|exe| omvix_windows::autostart::set_autostart(enabled, &exe));
                let copy_status = omvix_windows::autostart::query_installed_copy_status();
                if let Ok(mut st) = self.state.lock() {
                    st.installed_copy_status = copy_status;
                }
                record_result(
                    &self.state,
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
                        if self.cfg.general.start_with_windows
                            || omvix_windows::autostart::is_elevated_task_autostart_enabled()
                        {
                            omvix_windows::autostart::create_elevated_task(&installed)?;
                        }
                        Ok(())
                    });
                let copy_status = omvix_windows::autostart::query_installed_copy_status();
                if let Ok(mut st) = self.state.lock() {
                    st.installed_copy_status = copy_status;
                }
                record_result(&self.state, res, "Updated Program Files copy".to_string());
            }
            WorkerCommand::LaunchCustomCommand { command, args } => {
                let res = omvix_windows::unelevated::launch_unelevated(&command, &args);
                match res {
                    Ok(msg) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(msg, false);
                        }
                    }
                    Err(err) => {
                        tracing::warn!("Custom command failed: {err}");
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                }
            }
            WorkerCommand::RestoreAllDefaults => {
                self.safe_mode = None;
                self.hw_applied_recorded = false;
                if let Ok(mut cg) = self.crash_guard.lock() {
                    cg.clear_history();
                    let _ = cg.save_to_path(&self.crash_guard_path);
                }
                let prev_lang = self.cfg.general.language.clone();
                self.cfg = AppConfig::default();
                self.cfg.general.language = prev_lang;
                self.cfg.general.first_run_completed = true;
                self.app_profile_engine.reset();
                self.previous_active_rule = None;
                self.active_curve_override = None;
                self.prev_curve_applied = false;
                self.prev_curve_cool_hint = None;
                self.change_history.clear();
                self.last_committed_battery = BatteryConfig::default();
                self.last_committed_lighting = LightingConfig::default();
                self.last_committed_power_limits = (None, None);
                self.last_kbd_write_at = None;
                self.keyboard_dimmed_by_auto_profile = false;
                self.thermal_guard.reset();
                self.sensor_loss_guard.reset();
                self.retry_ctrl.reset();
                if self.bios_writes_allowed {
                    let _ = self.backends.thermal.set_fan_mode(FanMode::Auto, None);
                    let _ = self
                        .backends
                        .thermal
                        .set_performance_mode(PerformanceMode::Balanced);
                }
                let _ = self
                    .backends
                    .os
                    .set_win_power_overlay(WindowsPowerOverlay::Balanced);
                if omvix_windows::battery_service::is_service_installed()
                    || omvix_windows::battery_service::has_service_leftovers()
                {
                    let _ = omvix_windows::battery_service::uninstall_service_zero_trace();
                } else {
                    omvix_windows::autostart::remove_autostart_and_installed_copy();
                }
                self.last_applied_bat_mode = None;
                if self.bios_writes_allowed {
                    let _ = self
                        .backends
                        .battery
                        .set_charge_mode(BatteryChargeMode::Auto);
                    let _ = self.backends.kbd.set_backlight(
                        true,
                        100,
                        &[RgbColor {
                            r: 245,
                            g: 245,
                            b: 255,
                        }; 4],
                    );
                }
                self.bios_writes_allowed = is_bios_write_allowed(&self.board_id, None);
                let _ = self
                    .backends
                    .tuning
                    .set_cpu_boost_mode(CpuBoostMode::Aggressive);
                let disp = self.backends.display.get_display_mode().ok();
                if let Some(ref d) = disp {
                    if let Some(&max_hz) = d.supported_hz.iter().max() {
                        let _ = self.backends.display.set_refresh_rate(max_hz);
                    }
                }
                let refreshed_disp = self.backends.display.get_display_mode().ok();
                let refreshed_boost = self.backends.tuning.read_cpu_boost_mode().ok();
                let refreshed_bat = self.backends.battery.get_battery_status().ok();
                let refreshed_svc = omvix_windows::battery_service::query_service_info();
                let refreshed_copy = omvix_windows::autostart::query_installed_copy_status();
                let refreshed_overlay = self
                    .backends
                    .os
                    .read_win_power_overlay()
                    .or(Some(WindowsPowerOverlay::Balanced));
                if let Ok(mut st) = self.state.lock() {
                    st.safe_mode = None;
                    st.thermal_guard_active = false;
                    st.sensor_loss_active = false;
                    st.gpu_mux_reboot_pending = false;
                    st.bios_writes_allowed = self.bios_writes_allowed;
                    st.battery_service = refreshed_svc;
                    st.installed_copy_status = refreshed_copy;
                    st.active_app_profile = None;
                    st.active_app_profile_id = None;
                    st.effective_performance_mode = PerformanceMode::Balanced;
                    st.effective_fan_mode = FanMode::Auto;
                    st.effective_gpu_fan_mode = FanMode::Auto;
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
                    st.synced_config = Some(self.cfg.clone());
                    st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                    st.set_toast("All hardware & app settings restored to defaults", false);
                }
            }
            WorkerCommand::RebootSystem => {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let res =
                    std::process::Command::new(omvix_windows::paths::system_tool("shutdown.exe"))
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
                record_result(
                    &self.state,
                    res,
                    "Windows restart scheduled in 5 seconds...".to_string(),
                );
            }
            WorkerCommand::RefreshDiagnostics => {
                let res = self.backends.diag.collect_diagnostics();
                match res {
                    Ok(report) => {
                        let fan_conflicts = extract_fan_conflicts(&report.conflict_services);
                        if let Ok(mut st) = self.state.lock() {
                            st.security = report.security.clone();
                            st.conflict_services = report.conflict_services.clone();
                            st.fan_conflict_services = fan_conflicts;
                            st.diagnostics = Some(report);
                            st.set_toast("Diagnostics refreshed", false);
                        }
                    }
                    Err(err) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                }
            }
            WorkerCommand::ExportDiagnostics(path) => {
                match self.backends.diag.collect_diagnostics() {
                    Ok(report) => match serde_json::to_string_pretty(&report) {
                        Ok(json) => {
                            let res = fs::write(&path, json).map_err(Into::into);
                            record_result(
                                &self.state,
                                res,
                                format!("Exported diagnostics to {}", path.display()),
                            );
                        }
                        Err(err) => {
                            if let Ok(mut st) = self.state.lock() {
                                st.set_toast(err.to_string(), true);
                            }
                        }
                    },
                    Err(err) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                }
            }
            WorkerCommand::ApplyAutoProfileNow => {
                if self.safe_mode.is_some() {
                    if let Ok(mut st) = self.state.lock() {
                        st.set_toast("Blocked ApplyAutoProfileNow: Safe Mode is active", true);
                    }
                    return false;
                }
                let ac = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.telemetry.ac_connected)
                    .or_else(|| {
                        self.backends
                            .telemetry
                            .read_telemetry_with_options(false, false)
                            .ok()
                            .and_then(|s| s.ac_connected)
                    })
                    .unwrap_or(true);
                if let Some(action) =
                    self.auto_profile_engine
                        .evaluate(&self.cfg.auto_profile, ac, true)
                {
                    self.apply_auto_profile_action(action, ac);
                }
            }
            WorkerCommand::RefreshRunningProcesses => {
                let procs = self.backends.os.list_user_running_exes();
                if let Ok(mut st) = self.state.lock() {
                    st.running_exes_snapshot = procs;
                }
            }
            WorkerCommand::UndoLastChange => {
                if let Some(entry) = self.change_history.latest().cloned() {
                    if !check_hw_write_allowed(
                        self.safe_mode,
                        self.bios_writes_allowed,
                        entry.domain.requires_bios_write(),
                        &self.state,
                        "Undo",
                    ) {
                        return false;
                    }
                    if self.apply_undo_entry(&entry) {
                        let _ = self.change_history.pop_latest();
                        if let Ok(mut st) = self.state.lock() {
                            st.change_history = self.change_history.entries_vec();
                        }
                    }
                }
            }
            WorkerCommand::UndoChangeById(id) => {
                if let Some(entry) = self.change_history.peek_by_id(id).cloned() {
                    if !check_hw_write_allowed(
                        self.safe_mode,
                        self.bios_writes_allowed,
                        entry.domain.requires_bios_write(),
                        &self.state,
                        "Undo",
                    ) {
                        return false;
                    }
                    if self.apply_undo_entry(&entry) {
                        let _ = self.change_history.remove_by_id(id);
                        if let Ok(mut st) = self.state.lock() {
                            st.change_history = self.change_history.entries_vec();
                        }
                    }
                }
            }
            WorkerCommand::ClearChangeHistory => {
                self.change_history.clear();
                if let Ok(mut st) = self.state.lock() {
                    st.change_history.clear();
                }
            }
            WorkerCommand::CreateSupportBundle(out_path) => {
                if let Ok(mut st) = self.state.lock() {
                    if st.support_bundle_busy {
                        return false;
                    }
                    st.support_bundle_busy = true;
                    st.set_toast("Creating support bundle (.zip)...", false);
                }
                request_repaint_if_visible(ctx, window_visible);

                let diag = self.backends.diag.collect_diagnostics().ok();
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
                    config: self.cfg.clone(),
                    diagnostics: diag.clone(),
                    log_dir: self.cfg.logging.resolved_dir(),
                    crash_guard_path: Some(self.crash_guard_path.to_path_buf()),
                    extra_text_files,
                };
                let state_clone = Arc::clone(&self.state);
                let ctx_clone = ctx.clone();
                let vis_clone = Arc::clone(window_visible);
                thread::spawn(move || {
                    let res = build_support_bundle_zip(&out_path, &inputs);
                    if let Ok(mut st) = state_clone.lock() {
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
                    request_repaint_if_visible(&ctx_clone, &vis_clone);
                });
            }
            WorkerCommand::ExportTelemetryCsv(out_path) => {
                let rows: Vec<TelemetryCsvRow> = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|st| {
                        st.telemetry_csv_rows
                            .lock()
                            .ok()
                            .map(|r| r.iter().cloned().collect())
                    })
                    .unwrap_or_default();
                match write_telemetry_csv(&out_path, &rows) {
                    Ok(count) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.last_csv_export_path = Some(out_path);
                            st.set_toast(format!("Exported {count} CSV rows"), false);
                        }
                    }
                    Err(err) => {
                        tracing::warn!("Telemetry CSV export failed: {err}");
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                }
            }
            WorkerCommand::RecordBatteryHealthNow => {
                match self.backends.battery.get_battery_status() {
                    Ok(bat) => {
                        if let (Some(design), Some(full)) =
                            (bat.design_capacity_mwh, bat.full_charge_capacity_mwh)
                        {
                            let date_label = chrono::Local::now().format("%Y-%m-%d").to_string();
                            if self.battery_health_history.force_record_snapshot(
                                design,
                                full,
                                current_unix_secs() as i64,
                                date_label,
                            ) {
                                let save_res = self
                                    .battery_health_history
                                    .save_to_path(&self.battery_history_path);
                                if let Ok(mut st) = self.state.lock() {
                                    st.battery_status = Some(bat);
                                    st.battery_health_history =
                                        self.battery_health_history.records.clone();
                                    match save_res {
                                        Ok(()) => {
                                            st.set_toast("Logged battery health snapshot", false)
                                        }
                                        Err(err) => st.set_toast(err.to_string(), true),
                                    }
                                }
                            } else if let Ok(mut st) = self.state.lock() {
                                st.set_toast(
                                    "Battery capacity values were invalid for logging",
                                    true,
                                );
                            }
                        } else if let Ok(mut st) = self.state.lock() {
                            st.set_toast("Battery capacity telemetry unavailable", true);
                        }
                    }
                    Err(err) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.set_toast(err.to_string(), true);
                        }
                    }
                }
            }
            WorkerCommand::CheckForUpdates { silent } => {
                let busy = self
                    .state
                    .lock()
                    .map(|mut st| {
                        let active = matches!(
                            st.update_status,
                            UpdateUiStatus::Checking
                                | UpdateUiStatus::Downloading { .. }
                                | UpdateUiStatus::Applying { .. }
                        );
                        if active {
                            true
                        } else {
                            if !silent {
                                st.update_status = UpdateUiStatus::Checking;
                            }
                            false
                        }
                    })
                    .unwrap_or(true);
                if !busy {
                    request_repaint_if_visible(ctx, window_visible);
                    let state_bg = Arc::clone(&self.state);
                    let ctx_bg = ctx.clone();
                    let vis_bg = Arc::clone(window_visible);
                    thread::spawn(move || {
                        let res =
                            omvix_windows::updater::check_latest_release(env!("CARGO_PKG_VERSION"));
                        if let Ok(mut st) = state_bg.lock() {
                            match res {
                                Ok(
                                    omvix_windows::updater::ReleaseCheckOutcome::UpdateAvailable(
                                        rel,
                                    ),
                                ) => {
                                    st.set_toast(
                                        format!("Update available: v{}", rel.version),
                                        false,
                                    );
                                    st.update_status = UpdateUiStatus::UpdateAvailable(*rel);
                                }
                                Ok(omvix_windows::updater::ReleaseCheckOutcome::UpToDate {
                                    latest_tag,
                                }) => {
                                    st.update_status = UpdateUiStatus::UpToDate {
                                        checked_version: latest_tag,
                                        checked_at_utc: chrono::Utc::now()
                                            .format("%Y-%m-%d %H:%M UTC")
                                            .to_string(),
                                    };
                                    if !silent {
                                        st.set_toast("Omvix Hub is up to date", false);
                                    }
                                }
                                Ok(
                                    omvix_windows::updater::ReleaseCheckOutcome::NoReleasesPublished,
                                ) => {
                                    if !silent {
                                        st.update_status = UpdateUiStatus::NoReleasesPublished;
                                        st.set_toast(
                                            "No published releases found on GitHub yet",
                                            false,
                                        );
                                    }
                                }
                                Err(err) => {
                                    tracing::warn!("Update check failed: {err}");
                                    if !silent {
                                        st.update_status = UpdateUiStatus::Error(err.to_string());
                                        st.set_toast(err.to_string(), true);
                                    }
                                }
                            }
                        }
                        request_repaint_if_visible(&ctx_bg, &vis_bg);
                    });
                }
            }
            WorkerCommand::DownloadAndVerifyUpdate => {
                let rel_opt =
                    self.state
                        .lock()
                        .ok()
                        .and_then(|mut st| match st.update_status.clone() {
                            UpdateUiStatus::UpdateAvailable(rel) => {
                                st.update_status = UpdateUiStatus::Downloading {
                                    version: rel.version.clone(),
                                };
                                st.set_toast("Downloading & verifying update...", false);
                                Some(rel)
                            }
                            _ => None,
                        });
                if let Some(rel) = rel_opt {
                    request_repaint_if_visible(ctx, window_visible);
                    let state_bg = Arc::clone(&self.state);
                    let ctx_bg = ctx.clone();
                    let vis_bg = Arc::clone(window_visible);
                    thread::spawn(move || {
                        let res = omvix_windows::updater::download_and_verify_release(&rel);
                        if let Ok(mut st) = state_bg.lock() {
                            match res {
                                Ok(staged) => {
                                    st.update_status = UpdateUiStatus::ReadyToInstall(staged);
                                    st.set_toast("Update downloaded & SHA-256 verified", false);
                                }
                                Err(err) => {
                                    tracing::warn!("Update download/verify failed: {err}");
                                    st.update_status = UpdateUiStatus::Error(err.to_string());
                                    st.set_toast(err.to_string(), true);
                                }
                            }
                        }
                        request_repaint_if_visible(&ctx_bg, &vis_bg);
                    });
                }
            }
            WorkerCommand::ApplyStagedUpdate => {
                let staged_opt =
                    self.state
                        .lock()
                        .ok()
                        .and_then(|mut st| match st.update_status.clone() {
                            UpdateUiStatus::ReadyToInstall(staged) => {
                                st.update_status = UpdateUiStatus::Applying {
                                    version: staged.version.clone(),
                                };
                                Some(staged)
                            }
                            _ => None,
                        });
                if let Some(staged) = staged_opt {
                    request_repaint_if_visible(ctx, window_visible);
                    match omvix_windows::updater::apply_staged_update(
                        &staged,
                        self.cfg.general.allow_unsigned_sha256_updates,
                    ) {
                        Ok(exe_path) => {
                            if let Ok(mut st) = self.state.lock() {
                                st.pending_update_relaunch = Some(exe_path);
                                st.set_toast("Update applied — restarting Omvix Hub...", false);
                            }
                        }
                        Err(err) => {
                            tracing::warn!("Applying staged update failed: {err}");
                            if let Ok(mut st) = self.state.lock() {
                                st.update_status = if staged.binary_path.is_file() {
                                    UpdateUiStatus::ReadyToInstall(staged)
                                } else {
                                    UpdateUiStatus::Error(err.to_string())
                                };
                                st.set_toast(err.to_string(), true);
                            }
                        }
                    }
                    request_repaint_if_visible(ctx, window_visible);
                }
            }
            WorkerCommand::OpenReleasePage(url) => {
                if let Err(err) = omvix_windows::updater::open_verified_release_url(&url) {
                    if let Ok(mut st) = self.state.lock() {
                        st.set_toast(err.to_string(), true);
                    }
                }
            }
        }
        false
    }

    fn apply_off_state_safety(&self, reason: &str) {
        let t_total = Instant::now();
        let mut fan_ms = 0u128;
        let mut battery_ms = 0u128;
        let bios_allowed = self.bios_writes_allowed && self.safe_mode.is_none();
        let eff_cpu_fan = effective_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
        let eff_gpu_fan = effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());

        if bios_allowed
            && (eff_cpu_fan != FanMode::Auto
                || eff_gpu_fan != FanMode::Auto
                || self.thermal_guard.is_active())
        {
            let t_fan = Instant::now();
            if let Err(err) = self.backends.thermal.set_fan_mode(FanMode::Auto, None) {
                tracing::warn!(%reason, "Failed to restore BIOS Auto fan: {err}");
            } else {
                tracing::info!(%reason, "Restored BIOS Auto fan control");
            }
            fan_ms = t_fan.elapsed().as_millis();
        }

        if bios_allowed
            && !self.backends.os.is_battery_service_running()
            && self.cfg.battery.enabled
        {
            let t_bat = Instant::now();
            if !self.cfg.battery.hold_when_shutdown {
                if let Err(err) = self
                    .backends
                    .battery
                    .set_charge_mode(BatteryChargeMode::Auto)
                {
                    tracing::warn!(%reason, "Failed to restore normal battery charging: {err}");
                } else {
                    tracing::info!(%reason, "Restored normal battery charging (0x2B Auto)");
                }
            } else if !self
                .cfg
                .battery
                .is_travel_charge_active(chrono::Utc::now().timestamp())
            {
                let fast_pct = self.backends.os.read_fast_battery_pct().or_else(|| {
                    self.state.lock().ok().and_then(|s| {
                        s.telemetry
                            .battery_pct
                            .or_else(|| s.battery_status.as_ref().map(|b| b.charge_percent))
                    })
                });
                if let Some(pct) = fast_pct {
                    if pct >= 40 && pct >= self.cfg.battery.start_threshold_pct {
                        if let Err(err) = self
                            .backends
                            .battery
                            .set_charge_mode(BatteryChargeMode::Hold)
                        {
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

    fn colors_match_scaled(
        hw_zones: &[RgbColor],
        cfg_zones: &[RgbColor; 4],
        brightness_pct: u8,
    ) -> bool {
        let factor = brightness_pct.min(100) as f32 / 100.0;
        hw_zones.iter().enumerate().take(4).all(|(idx, hw)| {
            let src = cfg_zones[idx];
            let er = (src.r as f32 * factor).round().clamp(0.0, 255.0) as u8;
            let eg = (src.g as f32 * factor).round().clamp(0.0, 255.0) as u8;
            let eb = (src.b as f32 * factor).round().clamp(0.0, 255.0) as u8;
            hw.r.abs_diff(er) <= 2 && hw.g.abs_diff(eg) <= 2 && hw.b.abs_diff(eb) <= 2
        })
    }

    fn sync_keyboard_backlight_from_hw(&mut self, respect_grace: bool) -> bool {
        if respect_grace
            && self
                .last_kbd_write_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(1200))
        {
            return false;
        }
        let Ok(Some(hw)) = self.backends.kbd.get_backlight() else {
            return false;
        };

        if self.keyboard_dimmed_by_auto_profile {
            if !hw.enabled {
                // Expected auto-profile dim; keep user's saved cfg.lighting.enabled preference.
                return false;
            }
            // User manually re-enabled backlight (e.g. via Fn key) while auto-profile had dimmed it.
            self.keyboard_dimmed_by_auto_profile = false;
        }

        let mut changed = false;
        if self.cfg.lighting.enabled != hw.enabled {
            self.cfg.lighting.enabled = hw.enabled;
            changed = true;
        }

        let has_non_zero_color = hw.zones.iter().any(|c| c.r != 0 || c.g != 0 || c.b != 0);
        if hw.enabled
            && !hw.zones.is_empty()
            && has_non_zero_color
            && !Self::colors_match_scaled(
                &hw.zones,
                &self.cfg.lighting.zones,
                self.cfg.lighting.brightness_pct,
            )
        {
            for (dst, src) in self.cfg.lighting.zones.iter_mut().zip(hw.zones.iter()) {
                *dst = *src;
            }
            self.cfg.lighting.brightness_pct = 100;
            self.cfg.lighting.active_profile_id = None;
            changed = true;
        }

        if changed {
            self.last_committed_lighting = self.cfg.lighting.clone();
            if let Ok(mut st) = self.state.lock() {
                st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                st.synced_config = Some(self.cfg.clone());
            }
            tracing::info!(
                enabled = self.cfg.lighting.enabled,
                "Detected external keyboard backlight change; synchronized app state"
            );
        }
        changed
    }

    fn poll_external_hw_state_step(&mut self, respect_kbd_grace: bool) -> bool {
        let mut changed = self.sync_keyboard_backlight_from_hw(respect_kbd_grace);

        let disp = self.backends.display.get_display_mode().ok();
        let boost = self.backends.tuning.read_cpu_boost_mode().ok();
        let win_overlay = self.backends.os.read_win_power_overlay();
        if let Ok(mut st) = self.state.lock() {
            if let Some(d) = disp {
                let disp_changed = st.display_mode.as_ref().is_none_or(|cur| {
                    cur.current_hz != d.current_hz
                        || cur.width != d.width
                        || cur.height != d.height
                        || cur.supported_hz != d.supported_hz
                });
                if disp_changed {
                    st.display_mode = Some(d);
                    changed = true;
                }
            }
            if let Some(b) = boost {
                if st.cpu_boost != Some(b) {
                    st.cpu_boost = Some(b);
                    changed = true;
                }
            }
            if let Some(ov) = win_overlay {
                if st.active_win_power_mode != Some(ov) {
                    st.active_win_power_mode = Some(ov);
                    changed = true;
                }
            }
        }
        changed
    }

    fn apply_startup_saved_settings(&mut self) {
        if self.safe_mode.is_some() {
            tracing::info!("Safe Mode active; skipping saved startup hardware restore");
            return;
        }

        let (snap, cur_boost) = self
            .state
            .lock()
            .map(|st| (st.telemetry.clone(), st.cpu_boost))
            .unwrap_or_else(|_| (TelemetrySnapshot::default(), None));

        if !self.cfg.auto_profile.enabled && self.bios_writes_allowed {
            record_hw_write_attempt(
                &self.crash_guard,
                &self.crash_guard_path,
                &mut self.hw_applied_recorded,
            );
            let mode = self.cfg.thermal_fan.performance_mode;
            if let Err(err) = self.backends.thermal.set_performance_mode(mode) {
                tracing::warn!(?mode, "Startup performance mode restore failed: {err}");
            }
            self.sync_win_power_mode_if_enabled(mode);
        }

        if self.bios_writes_allowed {
            let eff_cpu = self.cfg.thermal_fan.fan_mode;
            let eff_gpu = effective_gpu_fan_mode(&self.cfg, None);
            if eff_cpu != FanMode::Auto || eff_gpu != FanMode::Auto {
                let cpu_curve = self.cfg.thermal_fan.custom_curve.clone();
                let gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
                let (hw_mode, cpu_target_pct, gpu_target_pct) =
                    resolve_independent_dual_fan_targets(
                        eff_cpu,
                        eff_gpu,
                        &snap,
                        &cpu_curve,
                        &gpu_curve,
                        Some(self.cfg.thermal_fan.manual_speed_pct),
                        Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
                        self.cfg.thermal_fan.link_cpu_gpu_fans,
                        &mut self.prev_curve_temp,
                        &mut self.prev_curve_speed,
                        &mut self.prev_gpu_curve_temp,
                        &mut self.prev_gpu_curve_speed,
                    );
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let cool_hint = cool_hint_from_snapshot(&snap);
                match self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                    hw_mode,
                    cpu_target_pct,
                    gpu_target_pct,
                    cool_hint,
                ) {
                    Ok(()) => {
                        self.prev_curve_applied =
                            eff_cpu == FanMode::CustomCurve || eff_gpu == FanMode::CustomCurve;
                        self.prev_curve_cool_hint = Some(cool_hint);
                    }
                    Err(err) => {
                        tracing::warn!("Startup fan mode restore failed: {err}");
                    }
                }
            }
        }

        if let Some(boost) = self.cfg.tuning.cpu_boost_mode {
            if cur_boost != Some(boost) {
                match self.backends.tuning.set_cpu_boost_mode(boost) {
                    Ok(()) => {
                        if let Ok(mut st) = self.state.lock() {
                            st.cpu_boost = Some(boost);
                        }
                    }
                    Err(err) => {
                        tracing::warn!(?boost, "Startup CPU boost restore failed: {err}");
                    }
                }
            }
        }

        if self.bios_writes_allowed
            && (self.cfg.tuning.pl1_watts.is_some() || self.cfg.tuning.pl2_watts.is_some())
        {
            record_hw_write_attempt(
                &self.crash_guard,
                &self.crash_guard_path,
                &mut self.hw_applied_recorded,
            );
            if let Err(err) = self
                .backends
                .tuning
                .set_power_limits(self.cfg.tuning.pl1_watts, self.cfg.tuning.pl2_watts)
            {
                tracing::warn!("Startup PL1/PL2 power limits restore failed: {err}");
            }
        }

        if self.cfg.battery.enabled {
            self.poll_battery_step();
        }
    }

    fn sync_win_power_mode_if_enabled(&self, mode: PerformanceMode) {
        if self.safe_mode.is_some() || !self.cfg.thermal_fan.sync_windows_power_mode {
            return;
        }
        let overlay = WindowsPowerOverlay::for_performance_mode(mode);
        match self.backends.os.set_win_power_overlay(overlay) {
            Ok(()) => {
                if let Ok(mut st) = self.state.lock() {
                    st.active_win_power_mode = Some(overlay);
                }
            }
            Err(err) => {
                tracing::warn!(?mode, ?overlay, "Windows Power Mode sync skipped: {err}");
            }
        }
    }

    fn apply_fan_override_if_safe(
        &mut self,
        cpu_fan_mode: FanMode,
        gpu_fan_mode: FanMode,
        cpu_curve: &FanCurve,
        manual_speed_pct: u8,
    ) {
        if !self.bios_writes_allowed
            || self.sensor_loss_guard.is_active()
            || self.thermal_guard.is_active()
        {
            return;
        }
        let snap = self
            .state
            .lock()
            .map(|st| st.telemetry.clone())
            .unwrap_or_default();
        let is_cool = cool_hint_from_snapshot(&snap);
        let gpu_curve = self.cfg.thermal_fan.gpu_custom_curve.clone();
        let link_fans =
            self.cfg.thermal_fan.link_cpu_gpu_fans || self.active_curve_override.is_some();
        let (hw_mode, cpu_target_pct, gpu_target_pct) = resolve_independent_dual_fan_targets(
            cpu_fan_mode,
            gpu_fan_mode,
            &snap,
            cpu_curve,
            &gpu_curve,
            Some(manual_speed_pct),
            Some(self.cfg.thermal_fan.gpu_manual_speed_pct),
            link_fans,
            &mut self.prev_curve_temp,
            &mut self.prev_curve_speed,
            &mut self.prev_gpu_curve_temp,
            &mut self.prev_gpu_curve_speed,
        );
        record_hw_write_attempt(
            &self.crash_guard,
            &self.crash_guard_path,
            &mut self.hw_applied_recorded,
        );
        if let Err(err) = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
            hw_mode,
            cpu_target_pct,
            gpu_target_pct,
            is_cool,
        ) {
            tracing::warn!(
                ?cpu_fan_mode,
                ?gpu_fan_mode,
                "Profile fan mode write failed: {err}"
            );
        }
    }

    fn apply_refresh_rate_if_set(&mut self, refresh_hz: Option<u32>) {
        let Some(hz) = refresh_hz else {
            return;
        };
        record_hw_write_attempt(
            &self.crash_guard,
            &self.crash_guard_path,
            &mut self.hw_applied_recorded,
        );
        if self.backends.display.set_refresh_rate(hz).is_ok() {
            if let Ok(disp) = self.backends.display.get_display_mode() {
                if let Ok(mut st) = self.state.lock() {
                    st.display_mode = Some(disp);
                }
            }
        }
    }

    fn apply_cpu_boost_if_set(&mut self, cpu_boost: Option<CpuBoostMode>) {
        let Some(boost) = cpu_boost else {
            return;
        };
        record_hw_write_attempt(
            &self.crash_guard,
            &self.crash_guard_path,
            &mut self.hw_applied_recorded,
        );
        if let Err(err) = self.backends.tuning.set_cpu_boost_mode(boost) {
            tracing::warn!(?boost, "Profile CPU boost write failed: {err}");
        } else if let Ok(mut st) = self.state.lock() {
            st.cpu_boost = Some(boost);
        }
    }

    fn apply_auto_profile_action(&mut self, action: AutoProfileAction, ac: bool) {
        if self.safe_mode.is_some() {
            return;
        }
        if self.app_profile_engine.active_rule().is_some() {
            self.app_profile_engine.update_baseline_for_ac_dc(&action);
            tracing::info!(
                ?action,
                "Deferred AC/DC auto-profile into active app-profile restore baseline"
            );
            return;
        }

        self.cfg.thermal_fan.performance_mode = action.performance_mode;
        let mut applied_perf = false;
        if self.bios_writes_allowed {
            record_hw_write_attempt(
                &self.crash_guard,
                &self.crash_guard_path,
                &mut self.hw_applied_recorded,
            );
            match self
                .backends
                .thermal
                .set_performance_mode(action.performance_mode)
            {
                Ok(()) => applied_perf = true,
                Err(err) => tracing::warn!("Auto-profile performance mode write failed: {err}"),
            }
        }
        self.sync_win_power_mode_if_enabled(action.performance_mode);

        if let Some(fan_mode) = action.fan_mode {
            self.cfg.thermal_fan.fan_mode = fan_mode;
            self.cfg.thermal_fan.gpu_fan_mode = fan_mode;
            self.active_curve_override = if fan_mode == FanMode::CustomCurve {
                action.curve_preset.map(|p| p.to_curve())
            } else {
                None
            };
            let active_curve = self
                .active_curve_override
                .clone()
                .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
            self.apply_fan_override_if_safe(
                fan_mode,
                fan_mode,
                &active_curve,
                self.cfg.thermal_fan.manual_speed_pct,
            );
        }

        self.apply_refresh_rate_if_set(action.refresh_hz);

        if self.bios_writes_allowed {
            if !self.keyboard_dimmed_by_auto_profile {
                self.sync_keyboard_backlight_from_hw(false);
            }
            record_hw_write_attempt(
                &self.crash_guard,
                &self.crash_guard_path,
                &mut self.hw_applied_recorded,
            );
            if action.dim_keyboard {
                if self.cfg.lighting.enabled {
                    match self
                        .backends
                        .kbd
                        .set_backlight(false, 0, &self.cfg.lighting.zones)
                    {
                        Ok(()) => {
                            self.keyboard_dimmed_by_auto_profile = true;
                            self.last_kbd_write_at = Some(Instant::now());
                        }
                        Err(err) => {
                            tracing::warn!("Auto-profile keyboard backlight write failed: {err}");
                        }
                    }
                }
            } else {
                if self.keyboard_dimmed_by_auto_profile && self.cfg.lighting.enabled {
                    match self.backends.kbd.set_backlight(
                        true,
                        self.cfg.lighting.brightness_pct,
                        &self.cfg.lighting.zones,
                    ) {
                        Ok(()) => {
                            self.last_kbd_write_at = Some(Instant::now());
                        }
                        Err(err) => {
                            tracing::warn!("Auto-profile keyboard backlight write failed: {err}");
                        }
                    }
                }
                self.keyboard_dimmed_by_auto_profile = false;
            }
        }

        if let Ok(mut st) = self.state.lock() {
            st.effective_performance_mode = self.cfg.thermal_fan.performance_mode;
            st.effective_fan_mode = self.cfg.thermal_fan.fan_mode;
            st.effective_gpu_fan_mode =
                effective_gpu_fan_mode(&self.cfg, self.app_profile_engine.active_rule());
            st.auto_applied_revision = st.auto_applied_revision.wrapping_add(1);
        }

        if applied_perf || !self.bios_writes_allowed {
            let src = if ac { "AC" } else { "Battery" };
            self.backends.os.show_osd(
                &format!("Auto-Profile ({src})"),
                &format!("{:?}", action.performance_mode),
                action.performance_mode,
            );
        }
    }

    fn apply_app_profile_transition(&mut self, transition: AppProfileTransition) {
        if self.safe_mode.is_some() {
            return;
        }
        match transition {
            AppProfileTransition::None => {}
            AppProfileTransition::Activate { rule } => {
                let prev_rule = self.previous_active_rule.take();
                let baseline_opt = self.app_profile_engine.saved_baseline().cloned();
                tracing::info!(
                    rule_id = %rule.id,
                    exe = %rule.exe_name,
                    perf = ?rule.performance_mode,
                    "Activating app-based hardware profile"
                );
                if self.bios_writes_allowed {
                    record_hw_write_attempt(
                        &self.crash_guard,
                        &self.crash_guard_path,
                        &mut self.hw_applied_recorded,
                    );
                    if let Err(err) = self
                        .backends
                        .thermal
                        .set_performance_mode(rule.performance_mode)
                    {
                        tracing::warn!("App profile performance write failed: {err}");
                    }
                }
                self.sync_win_power_mode_if_enabled(rule.performance_mode);

                let (eff_cpu_fan, eff_gpu_fan) = if let Some(fan_mode) = rule.fan_mode {
                    self.active_curve_override = if fan_mode == FanMode::CustomCurve {
                        rule.curve_preset.map(|p| p.to_curve())
                    } else {
                        None
                    };
                    let active_curve = self
                        .active_curve_override
                        .clone()
                        .unwrap_or_else(|| self.cfg.thermal_fan.custom_curve.clone());
                    self.apply_fan_override_if_safe(
                        fan_mode,
                        fan_mode,
                        &active_curve,
                        self.cfg.thermal_fan.manual_speed_pct,
                    );
                    (fan_mode, fan_mode)
                } else if prev_rule.as_ref().is_some_and(|p| p.fan_mode.is_some()) {
                    self.active_curve_override = None;
                    if let Some(ref baseline) = baseline_opt {
                        let rest_gpu = if baseline.link_cpu_gpu_fans {
                            baseline.fan_mode
                        } else {
                            baseline.gpu_fan_mode
                        };
                        self.cfg.thermal_fan.gpu_custom_curve = baseline.gpu_custom_curve.clone();
                        self.cfg.thermal_fan.gpu_manual_speed_pct = baseline.gpu_manual_speed_pct;
                        self.apply_fan_override_if_safe(
                            baseline.fan_mode,
                            rest_gpu,
                            &baseline.custom_curve,
                            baseline.manual_speed_pct,
                        );
                        (baseline.fan_mode, rest_gpu)
                    } else {
                        (
                            effective_fan_mode(&self.cfg, None),
                            effective_gpu_fan_mode(&self.cfg, None),
                        )
                    }
                } else {
                    (
                        effective_fan_mode(&self.cfg, None),
                        effective_gpu_fan_mode(&self.cfg, None),
                    )
                };

                if rule.refresh_hz.is_some() {
                    self.apply_refresh_rate_if_set(rule.refresh_hz);
                } else if prev_rule.as_ref().is_some_and(|p| p.refresh_hz.is_some()) {
                    self.apply_refresh_rate_if_set(
                        baseline_opt.as_ref().and_then(|b| b.refresh_hz),
                    );
                }
                if rule.cpu_boost_mode.is_some() {
                    self.apply_cpu_boost_if_set(rule.cpu_boost_mode);
                } else if prev_rule
                    .as_ref()
                    .is_some_and(|p| p.cpu_boost_mode.is_some())
                {
                    self.apply_cpu_boost_if_set(
                        baseline_opt.as_ref().and_then(|b| b.cpu_boost_mode),
                    );
                }

                self.previous_active_rule = Some(rule.clone());
                self.prev_curve_applied = false;
                let label = rule.display_label().to_string();
                if let Ok(mut st) = self.state.lock() {
                    st.active_app_profile = Some(label.clone());
                    st.active_app_profile_id = Some(rule.id.clone());
                    st.effective_performance_mode = rule.performance_mode;
                    st.effective_fan_mode = eff_cpu_fan;
                    st.effective_gpu_fan_mode = eff_gpu_fan;
                    st.set_toast(format!("App profile active: {label}"), false);
                }
                self.backends.os.show_osd(
                    "App Profile",
                    &format!("{label} · {:?}", rule.performance_mode),
                    rule.performance_mode,
                );
            }
            AppProfileTransition::Restore(baseline) => {
                tracing::info!(
                    ?baseline.performance_mode,
                    ?baseline.fan_mode,
                    ?baseline.gpu_fan_mode,
                    "Restoring baseline after app exit"
                );
                self.previous_active_rule = None;
                self.active_curve_override = None;
                self.prev_curve_applied = false;
                self.cfg.thermal_fan.performance_mode = baseline.performance_mode;
                self.cfg.thermal_fan.fan_mode = baseline.fan_mode;
                self.cfg.thermal_fan.gpu_fan_mode = baseline.gpu_fan_mode;
                self.cfg.thermal_fan.manual_speed_pct = baseline.manual_speed_pct;
                self.cfg.thermal_fan.gpu_manual_speed_pct = baseline.gpu_manual_speed_pct;
                self.cfg.thermal_fan.custom_curve = baseline.custom_curve.clone();
                self.cfg.thermal_fan.gpu_custom_curve = baseline.gpu_custom_curve.clone();
                self.cfg.thermal_fan.link_cpu_gpu_fans = baseline.link_cpu_gpu_fans;

                if self.bios_writes_allowed {
                    record_hw_write_attempt(
                        &self.crash_guard,
                        &self.crash_guard_path,
                        &mut self.hw_applied_recorded,
                    );
                    if let Err(err) = self
                        .backends
                        .thermal
                        .set_performance_mode(baseline.performance_mode)
                    {
                        tracing::warn!("App profile baseline performance restore failed: {err}");
                    }
                }
                self.sync_win_power_mode_if_enabled(baseline.performance_mode);

                let restore_gpu_mode = if baseline.link_cpu_gpu_fans {
                    baseline.fan_mode
                } else {
                    baseline.gpu_fan_mode
                };
                self.apply_fan_override_if_safe(
                    baseline.fan_mode,
                    restore_gpu_mode,
                    &baseline.custom_curve,
                    baseline.manual_speed_pct,
                );
                self.apply_refresh_rate_if_set(baseline.refresh_hz);
                self.apply_cpu_boost_if_set(baseline.cpu_boost_mode);

                if let Ok(mut st) = self.state.lock() {
                    st.active_app_profile = None;
                    st.active_app_profile_id = None;
                    st.effective_performance_mode = baseline.performance_mode;
                    st.effective_fan_mode = baseline.fan_mode;
                    st.effective_gpu_fan_mode = restore_gpu_mode;
                    st.auto_applied_revision = st.auto_applied_revision.wrapping_add(1);
                    st.synced_config = Some(self.cfg.clone());
                    st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                    st.set_toast("Restored baseline profile", false);
                }
                self.backends.os.show_osd(
                    "Profile Restored",
                    &format!("{:?}", baseline.performance_mode),
                    baseline.performance_mode,
                );
            }
        }
    }

    fn restore_ec_default_power_limits_via_profile(&self) -> Result<()> {
        let effective_perf = self
            .state
            .lock()
            .map(|st| st.effective_performance_mode)
            .unwrap_or(self.cfg.thermal_fan.performance_mode);
        // TODO(hw-verify): confirm whether HP WMI 0x29 has a dedicated default-reset byte or relies
        // on re-applying the active thermal profile (0x1A/0x30) to restore EC default PL1/PL2.
        let res = self.backends.thermal.set_performance_mode(effective_perf);
        if res.is_ok() {
            self.sync_win_power_mode_if_enabled(effective_perf);
        }
        res
    }

    fn apply_undo_entry(&mut self, entry: &HardwareChangeEntry) -> bool {
        let mut guard_blocked = false;
        let res: Result<()> = match entry.revert.clone() {
            RevertPayload::PerformanceMode(mode) => {
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let r = self.backends.thermal.set_performance_mode(mode);
                if r.is_ok() {
                    self.cfg.thermal_fan.performance_mode = mode;
                    self.sync_win_power_mode_if_enabled(mode);
                    if let Ok(mut st) = self.state.lock() {
                        st.effective_performance_mode = mode;
                    }
                }
                r
            }
            RevertPayload::FanMode {
                mode,
                gpu_mode,
                manual_speed_pct,
                gpu_manual_speed_pct,
                custom_curve,
                gpu_custom_curve,
                link_cpu_gpu_fans,
            } => {
                let target_gpu_mode = if link_cpu_gpu_fans { mode } else { gpu_mode };
                if self.sensor_loss_guard.is_active()
                    && (mode != FanMode::Auto || target_gpu_mode != FanMode::Auto)
                {
                    guard_blocked = true;
                    Err(omvix_core::OmvixError::Config(
                        "Thermal sensors lost: Auto fallback active until sensors recover"
                            .to_string(),
                    ))
                } else if self.thermal_guard.is_active() {
                    guard_blocked = true;
                    Err(omvix_core::OmvixError::Config(
                        "Thermal Guard active: Max Fan remains engaged until temperatures drop"
                            .to_string(),
                    ))
                } else {
                    record_hw_write_attempt(
                        &self.crash_guard,
                        &self.crash_guard_path,
                        &mut self.hw_applied_recorded,
                    );
                    let snap = self
                        .state
                        .lock()
                        .map(|st| st.telemetry.clone())
                        .unwrap_or_default();
                    let is_cool = cool_hint_from_snapshot(&snap);
                    let mut next_prev_temp = self.prev_curve_temp;
                    let mut next_prev_speed = self.prev_curve_speed;
                    let mut next_prev_gpu_temp = self.prev_gpu_curve_temp;
                    let mut next_prev_gpu_speed = self.prev_gpu_curve_speed;
                    let (hw_mode, cpu_target_pct, gpu_target_pct) =
                        resolve_independent_dual_fan_targets(
                            mode,
                            target_gpu_mode,
                            &snap,
                            &custom_curve,
                            &gpu_custom_curve,
                            Some(manual_speed_pct),
                            Some(gpu_manual_speed_pct),
                            link_cpu_gpu_fans,
                            &mut next_prev_temp,
                            &mut next_prev_speed,
                            &mut next_prev_gpu_temp,
                            &mut next_prev_gpu_speed,
                        );
                    let r = self.backends.thermal.set_dual_fan_mode_with_cool_hint(
                        hw_mode,
                        cpu_target_pct,
                        gpu_target_pct,
                        is_cool,
                    );
                    if r.is_ok() {
                        self.cfg.thermal_fan.fan_mode = mode;
                        self.cfg.thermal_fan.gpu_fan_mode = target_gpu_mode;
                        self.cfg.thermal_fan.manual_speed_pct = manual_speed_pct;
                        self.cfg.thermal_fan.gpu_manual_speed_pct = gpu_manual_speed_pct;
                        self.cfg.thermal_fan.custom_curve = custom_curve;
                        self.cfg.thermal_fan.gpu_custom_curve = gpu_custom_curve;
                        self.cfg.thermal_fan.link_cpu_gpu_fans = link_cpu_gpu_fans;
                        self.prev_curve_applied = true;
                        self.prev_curve_cool_hint = Some(is_cool);
                        self.active_curve_override = None;
                        self.retry_ctrl.reset();
                        self.prev_curve_temp = next_prev_temp;
                        self.prev_curve_speed = next_prev_speed;
                        self.prev_gpu_curve_temp = next_prev_gpu_temp;
                        self.prev_gpu_curve_speed = next_prev_gpu_speed;
                        if let Ok(mut st) = self.state.lock() {
                            st.effective_fan_mode = mode;
                            st.effective_gpu_fan_mode = target_gpu_mode;
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
                travel_charge_until_utc,
                active_profile_id,
            } => {
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let mut next_battery = self.cfg.battery.clone();
                next_battery.enabled = enabled;
                next_battery.mode = mode;
                next_battery.stop_threshold_pct = stop_threshold_pct;
                next_battery.start_threshold_pct = start_threshold_pct;
                next_battery.hold_when_shutdown = hold_when_shutdown;
                next_battery.one_time_full_charge = one_time_full_charge;
                next_battery.travel_charge_until_utc = travel_charge_until_utc;
                next_battery.active_profile_id = active_profile_id;
                match self.backends.battery.get_battery_status() {
                    Ok(bat) => {
                        let target = self.battery_engine.evaluate(
                            &mut next_battery,
                            bat.charge_percent,
                            bat.ac_connected,
                        );
                        let r = self.backends.battery.set_charge_mode(target);
                        if r.is_ok() {
                            self.cfg.battery = next_battery;
                            let _ = self
                                .backends
                                .os
                                .sync_battery_service_config(&self.cfg.battery);
                            self.last_applied_bat_mode = Some(target);
                            self.last_committed_battery = self.cfg.battery.clone();
                            if let Ok(refreshed) = self.backends.battery.get_battery_status() {
                                if let Ok(mut st) = self.state.lock() {
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
                let r = self.backends.display.set_refresh_rate(hz);
                if r.is_ok() {
                    if let Ok(disp) = self.backends.display.get_display_mode() {
                        if let Ok(mut st) = self.state.lock() {
                            st.display_mode = Some(disp);
                        }
                    }
                }
                r
            }
            RevertPayload::CpuBoost(boost) => {
                let r = self.backends.tuning.set_cpu_boost_mode(boost);
                if r.is_ok() {
                    self.cfg.tuning.cpu_boost_mode = Some(boost);
                    if let Ok(mut st) = self.state.lock() {
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
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let r = self
                    .backends
                    .kbd
                    .set_backlight(enabled, brightness_pct, &zones);
                if r.is_ok() {
                    self.cfg.lighting.enabled = enabled;
                    self.cfg.lighting.brightness_pct = brightness_pct;
                    self.cfg.lighting.zones = zones;
                    self.last_committed_lighting = self.cfg.lighting.clone();
                    self.last_kbd_write_at = Some(Instant::now());
                    self.keyboard_dimmed_by_auto_profile = false;
                }
                r
            }
            RevertPayload::PowerLimits {
                pl1_watts,
                pl2_watts,
            } => {
                record_hw_write_attempt(
                    &self.crash_guard,
                    &self.crash_guard_path,
                    &mut self.hw_applied_recorded,
                );
                let needs_reset = (pl1_watts.is_none()
                    && self.last_committed_power_limits.0.is_some())
                    || (pl2_watts.is_none() && self.last_committed_power_limits.1.is_some())
                    || (pl1_watts.is_none() && pl2_watts.is_none());
                let mut r = if needs_reset {
                    self.restore_ec_default_power_limits_via_profile()
                } else {
                    Ok(())
                };
                if r.is_ok() && (pl1_watts.is_some() || pl2_watts.is_some()) {
                    r = self.backends.tuning.set_power_limits(pl1_watts, pl2_watts);
                }
                if r.is_ok() {
                    self.cfg.tuning.pl1_watts = pl1_watts;
                    self.cfg.tuning.pl2_watts = pl2_watts;
                    self.last_committed_power_limits = (pl1_watts, pl2_watts);
                }
                r
            }
        };

        let ok = res.is_ok();
        if let Ok(mut st) = self.state.lock() {
            match res {
                Ok(()) => {
                    st.synced_config_rev = st.synced_config_rev.wrapping_add(1);
                    st.synced_config = Some(self.cfg.clone());
                    st.set_toast(format!("Undid change -> {}", entry.old_summary), false);
                }
                Err(err) => {
                    if !guard_blocked {
                        tracing::warn!("Undo failed: {err}");
                    }
                    st.set_toast(err.to_string(), true);
                }
            }
        }
        ok
    }
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
fn cpu_curve_temp_c(snap: &TelemetrySnapshot, link_fans: bool) -> Option<f32> {
    if link_fans || snap.cpu_temp_c.is_none() {
        effective_fan_curve_temp_c(
            snap.cpu_temp_c,
            snap.gpu_temp_c,
            snap.chassis_temp_c,
            snap.cpu_power_w,
        )
    } else {
        effective_fan_curve_temp_c(snap.cpu_temp_c, None, snap.chassis_temp_c, snap.cpu_power_w)
    }
}

#[cfg(windows)]
fn gpu_curve_temp_c(snap: &TelemetrySnapshot) -> Option<f32> {
    if !snap.gpu_sleeping {
        if let Some(gt) = snap.gpu_temp_c {
            return Some(gt);
        }
    }
    snap.chassis_temp_c
        .or_else(|| snap.cpu_temp_c.map(|c| (c - 10.0).max(35.0)))
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn resolve_independent_dual_fan_targets(
    cpu_mode: FanMode,
    gpu_mode: FanMode,
    snap: &TelemetrySnapshot,
    cpu_curve: &FanCurve,
    gpu_curve: &FanCurve,
    cpu_manual_pct: Option<u8>,
    gpu_manual_pct: Option<u8>,
    link_fans: bool,
    prev_cpu_temp: &mut f32,
    prev_cpu_speed: &mut u8,
    prev_gpu_temp: &mut f32,
    prev_gpu_speed: &mut u8,
) -> (FanMode, Option<u8>, Option<u8>) {
    let eff_gpu_mode = if link_fans { cpu_mode } else { gpu_mode };
    if cpu_mode == FanMode::Auto && eff_gpu_mode == FanMode::Auto {
        return (FanMode::Auto, None, None);
    }
    if cpu_mode == FanMode::Max && eff_gpu_mode == FanMode::Max {
        return (FanMode::Max, None, None);
    }

    let cpu_pct = match cpu_mode {
        FanMode::Max => {
            *prev_cpu_speed = 100;
            Some(100)
        }
        FanMode::Manual => {
            let p = cpu_manual_pct.unwrap_or(50).clamp(0, 100);
            *prev_cpu_speed = p;
            Some(p)
        }
        FanMode::CustomCurve => {
            if let Some(temp) = cpu_curve_temp_c(snap, link_fans) {
                let pct = cpu_curve.evaluate(temp, temp, 0);
                *prev_cpu_temp = temp;
                *prev_cpu_speed = pct;
                Some(pct)
            } else {
                let p = cpu_manual_pct.unwrap_or(45).clamp(0, 100);
                *prev_cpu_speed = p;
                Some(p)
            }
        }
        FanMode::Auto => {
            let auto_curve = FanCurve::preset_ogh_like();
            if let Some(temp) = cpu_curve_temp_c(snap, false) {
                let pct = auto_curve.evaluate(temp, temp, 0);
                *prev_cpu_temp = temp;
                *prev_cpu_speed = pct;
                Some(pct)
            } else {
                *prev_cpu_speed = 45;
                Some(45)
            }
        }
    };

    let gpu_pct = if link_fans {
        if let Some(p) = cpu_pct {
            *prev_gpu_speed = p;
        }
        cpu_pct
    } else {
        match eff_gpu_mode {
            FanMode::Max => {
                *prev_gpu_speed = 100;
                Some(100)
            }
            FanMode::Manual => {
                let p = gpu_manual_pct
                    .or(cpu_manual_pct)
                    .unwrap_or(50)
                    .clamp(0, 100);
                *prev_gpu_speed = p;
                Some(p)
            }
            FanMode::CustomCurve => {
                if let Some(gt) = gpu_curve_temp_c(snap) {
                    let pct = gpu_curve.evaluate(gt, gt, 0);
                    *prev_gpu_temp = gt;
                    *prev_gpu_speed = pct;
                    Some(pct)
                } else {
                    let fallback = gpu_manual_pct.or(cpu_pct);
                    if let Some(p) = fallback {
                        *prev_gpu_speed = p;
                    }
                    fallback
                }
            }
            FanMode::Auto => {
                let auto_curve = FanCurve::preset_ogh_like();
                if let Some(gt) = gpu_curve_temp_c(snap) {
                    let pct = auto_curve.evaluate(gt, gt, 0);
                    *prev_gpu_temp = gt;
                    *prev_gpu_speed = pct;
                    Some(pct)
                } else {
                    *prev_gpu_speed = 45;
                    Some(45)
                }
            }
        }
    };

    let hw_mode = if cpu_mode == FanMode::CustomCurve
        || eff_gpu_mode == FanMode::CustomCurve
        || cpu_mode == FanMode::Auto
        || eff_gpu_mode == FanMode::Auto
    {
        FanMode::CustomCurve
    } else {
        FanMode::Manual
    };
    (hw_mode, cpu_pct, gpu_pct)
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
fn format_fan_summary(cpu_mode: FanMode, gpu_mode: FanMode, cpu_pct: u8, gpu_pct: u8) -> String {
    if cpu_mode == gpu_mode {
        match cpu_mode {
            FanMode::Manual if cpu_pct == gpu_pct => format!("Manual ({cpu_pct}%)"),
            FanMode::Manual => format!("Manual (CPU {cpu_pct}% / GPU {gpu_pct}%)"),
            other => format!("{other:?}"),
        }
    } else {
        let fmt_ch = |m: FanMode, p: u8| match m {
            FanMode::Manual => format!("Manual {p}%"),
            other => format!("{other:?}"),
        };
        format!(
            "CPU {} / GPU {}",
            fmt_ch(cpu_mode, cpu_pct),
            fmt_ch(gpu_mode, gpu_pct)
        )
    }
}

fn should_skip_battery_apply(
    current_mode: BatteryChargeMode,
    last_applied: Option<BatteryChargeMode>,
    target: BatteryChargeMode,
) -> bool {
    current_mode != BatteryChargeMode::Unknown
        && target != BatteryChargeMode::Unknown
        && current_mode == target
        && last_applied == Some(target)
}

#[cfg(windows)]
fn format_battery_summary(bat: &BatteryConfig) -> String {
    if !bat.enabled || bat.mode == BatteryChargeMode::Auto {
        "Auto (100%)".to_string()
    } else if bat.is_travel_charge_active(chrono::Utc::now().timestamp()) {
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
    let f = |v: Option<u16>| {
        v.map(|w| format!("{w}W"))
            .unwrap_or_else(|| "Unchanged".into())
    };
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
        WorkerCommand::PollExternalHardwareState => Some(7),
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
            | WorkerCommand::RestoreDefaultPowerLimits { .. }
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
            | WorkerCommand::ExportTelemetryCsv(_)
            | WorkerCommand::RecordBatteryHealthNow
            | WorkerCommand::CheckForUpdates { .. }
            | WorkerCommand::DownloadAndVerifyUpdate
            | WorkerCommand::ApplyStagedUpdate
            | WorkerCommand::OpenReleasePage(_)
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

    let mut merged_fan_gpu_mode: Option<FanMode> = None;
    let mut merged_fan_cpu_pct: Option<u8> = None;
    let mut merged_fan_gpu_pct: Option<u8> = None;
    let start_out = out.len();
    for (idx, mut cmd) in seg.into_iter().enumerate() {
        if let Some(kind) = latest_wins_kind(&cmd) {
            if last_idx_by_kind[kind as usize] != Some(idx) {
                if let WorkerCommand::SetFanMode {
                    gpu_mode,
                    manual_speed_pct,
                    gpu_manual_speed_pct,
                    ..
                } = cmd
                {
                    if gpu_mode.is_some() {
                        merged_fan_gpu_mode = gpu_mode;
                    }
                    if manual_speed_pct.is_some() {
                        merged_fan_cpu_pct = manual_speed_pct;
                    }
                    if gpu_manual_speed_pct.is_some() {
                        merged_fan_gpu_pct = gpu_manual_speed_pct;
                    }
                }
                continue;
            }
            if let WorkerCommand::SetFanMode {
                ref mut gpu_mode,
                ref mut manual_speed_pct,
                ref mut gpu_manual_speed_pct,
                ..
            } = cmd
            {
                if gpu_mode.is_none() {
                    *gpu_mode = merged_fan_gpu_mode;
                }
                if manual_speed_pct.is_none() {
                    *manual_speed_pct = merged_fan_cpu_pct;
                }
                if gpu_manual_speed_pct.is_none() {
                    *gpu_manual_speed_pct = merged_fan_gpu_pct;
                }
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
                gpu_mode: Some(FanMode::Manual),
                manual_speed_pct: Some(30),
                gpu_manual_speed_pct: Some(30),
            },
            WorkerCommand::SetFanMode {
                mode: FanMode::Manual,
                gpu_mode: Some(FanMode::Manual),
                manual_speed_pct: Some(55),
                gpu_manual_speed_pct: Some(45),
            },
            WorkerCommand::SetPowerLimits {
                pl1: Some(45),
                pl2: Some(90),
            },
            WorkerCommand::UpdateConfig(cfg1),
            WorkerCommand::RestoreAllDefaults,
            WorkerCommand::SetFanMode {
                mode: FanMode::Auto,
                gpu_mode: Some(FanMode::Auto),
                manual_speed_pct: None,
                gpu_manual_speed_pct: None,
            },
            WorkerCommand::SetFanMode {
                mode: FanMode::Max,
                gpu_mode: Some(FanMode::Max),
                manual_speed_pct: None,
                gpu_manual_speed_pct: None,
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
                gpu_mode: Some(FanMode::Manual),
                manual_speed_pct: Some(55),
                gpu_manual_speed_pct: Some(45),
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
                gpu_mode: Some(FanMode::Max),
                manual_speed_pct: None,
                gpu_manual_speed_pct: None,
            }
        ));
    }

    #[test]
    fn simulated_sensor_loss_triggers_auto_pauses_keepalive_and_restores_mode() {
        let mut sl = SensorLossGuard::new(DEFAULT_SENSOR_LOSS_TIMEOUT_SECS);
        let mut applied: Vec<(f64, FanMode, Option<u8>, Option<u8>)> = Vec::new();
        let user_mode = FanMode::CustomCurve;
        let user_cpu_pct = Some(64u8);
        let user_gpu_pct = Some(48u8);

        // 0..5s without any temperature reading -> stays normal, keep-alive allowed
        for sec in [0.0, 2.0, 5.0] {
            let (dec, ka_suppressed) = step_sensor_loss_cycle(
                &mut sl,
                false,
                sec,
                true,
                false,
                user_mode,
                user_cpu_pct,
                user_gpu_pct,
                false,
                |mode, cpu_pct, gpu_pct, _| {
                    applied.push((sec, mode, cpu_pct, gpu_pct));
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
            user_cpu_pct,
            user_gpu_pct,
            false,
            |mode, cpu_pct, gpu_pct, _| {
                applied.push((6.0, mode, cpu_pct, gpu_pct));
                Ok(())
            },
        );
        assert_eq!(dec6, SensorLossDecision::EngageAutoFallback);
        assert!(ka_suppressed6);
        assert!(!should_run_keep_alive(true, ka_suppressed6, true));
        assert_eq!(applied, vec![(6.0, FanMode::Auto, None, None)]);

        // 8s sensor recovers -> restores user's CustomCurve & resumes keep-alive
        let (dec8, ka_suppressed8) = step_sensor_loss_cycle(
            &mut sl,
            true,
            8.0,
            true,
            false,
            user_mode,
            user_cpu_pct,
            user_gpu_pct,
            true,
            |mode, cpu_pct, gpu_pct, _| {
                applied.push((8.0, mode, cpu_pct, gpu_pct));
                Ok(())
            },
        );
        assert_eq!(dec8, SensorLossDecision::RestoreUserMode);
        assert!(!ka_suppressed8);
        assert!(should_run_keep_alive(true, ka_suppressed8, true));
        assert_eq!(
            applied,
            vec![
                (6.0, FanMode::Auto, None, None),
                (8.0, FanMode::CustomCurve, Some(64), Some(48))
            ]
        );
    }

    #[test]
    fn should_skip_battery_apply_never_matches_unknown() {
        assert!(!should_skip_battery_apply(
            BatteryChargeMode::Unknown,
            Some(BatteryChargeMode::Inhibit),
            BatteryChargeMode::Inhibit
        ));
        assert!(!should_skip_battery_apply(
            BatteryChargeMode::Unknown,
            Some(BatteryChargeMode::Unknown),
            BatteryChargeMode::Unknown
        ));
        assert!(!should_skip_battery_apply(
            BatteryChargeMode::Auto,
            Some(BatteryChargeMode::Auto),
            BatteryChargeMode::Inhibit
        ));
        assert!(should_skip_battery_apply(
            BatteryChargeMode::Inhibit,
            Some(BatteryChargeMode::Inhibit),
            BatteryChargeMode::Inhibit
        ));
    }
}

#[cfg(all(test, windows))]
#[path = "worker_tests.rs"]
mod integration_tests;
