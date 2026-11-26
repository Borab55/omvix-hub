use super::*;
use chrono::Utc;
use omvix_core::app_profiles::AppProfileRule;
use omvix_core::OmvixError;
use omvix_core::{FanCurvePreset, SystemIdentity};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Debug, Clone)]
struct MockHardwareState {
    diag_report: DiagnosticReport,
    telemetry_snap: TelemetrySnapshot,
    pawnio_active: bool,
    reload_pawnio_result: bool,
    perf_mode: PerformanceMode,
    fan_mode: FanMode,
    fail_max_fan: bool,
    fan_writes: Vec<(FanMode, Option<u8>, bool)>,
    dual_fan_writes: Vec<(FanMode, Option<u8>, Option<u8>, bool)>,
    keep_alive_count: usize,
    battery_status: BatteryStatus,
    battery_mode_writes: Vec<BatteryChargeMode>,
    lighting_writes: Vec<(bool, u8, [RgbColor; 4])>,
    display_mode: DisplayModeInfo,
    refresh_writes: Vec<u32>,
    gpu_mode: GpuMode,
    gpu_writes: Vec<GpuMode>,
    cpu_boost: CpuBoostMode,
    cpu_boost_writes: Vec<CpuBoostMode>,
    power_limits_writes: Vec<(Option<u16>, Option<u16>)>,
    toasts: Vec<(String, String, WorkerToastSeverity)>,
    osds: Vec<(String, String, PerformanceMode)>,
    win_overlay: Option<WindowsPowerOverlay>,
    win_overlay_writes: Vec<WindowsPowerOverlay>,
    running_exes_lower: HashSet<String>,
    user_running_exes: Vec<String>,
    battery_service_running: bool,
    battery_service_cfg: Option<BatteryConfig>,
    fast_battery_pct: Option<u8>,
    event_log: Vec<String>,
}

impl Default for MockHardwareState {
    fn default() -> Self {
        Self {
            diag_report: DiagnosticReport {
                generated_at: Utc::now(),
                app_version: "0.5.1".to_string(),
                identity: SystemIdentity {
                    manufacturer: "HP".to_string(),
                    model: "Victus by HP 16".to_string(),
                    system_family: "Victus".to_string(),
                    board_id: "8C99".to_string(),
                    bios_version: "F.20".to_string(),
                    cpu_name: "Intel Core i7-14700HX".to_string(),
                    gpu_names: vec!["NVIDIA RTX 4060".to_string()],
                    os_name: "Windows 11".to_string(),
                },
                matched_board: None,
                security: SecurityStatus {
                    is_admin: true,
                    vbs_status: Some(0),
                    hvci_running: false,
                    pawn_io_installed: false,
                    undervolt_note: String::new(),
                },
                wmi_hpqbintm_available: true,
                wmi_hpqbevnt_available: true,
                wmi_error: None,
                conflict_services: Vec::new(),
                probes: Vec::new(),
            },
            telemetry_snap: TelemetrySnapshot {
                cpu_temp_c: Some(52.0),
                gpu_temp_c: Some(46.0),
                chassis_temp_c: Some(38.0),
                cpu_usage_pct: Some(18.0),
                fan_rpms: vec![2200, 2200],
                fan_max_rpms: vec![5200, 5200],
                ac_connected: Some(true),
                battery_pct: Some(82),
                ..Default::default()
            },
            pawnio_active: false,
            reload_pawnio_result: false,
            perf_mode: PerformanceMode::Balanced,
            fan_mode: FanMode::Auto,
            fail_max_fan: false,
            fan_writes: Vec::new(),
            dual_fan_writes: Vec::new(),
            keep_alive_count: 0,
            battery_status: BatteryStatus {
                present: true,
                ac_connected: true,
                charging: true,
                charge_percent: 82,
                design_capacity_mwh: Some(70_000),
                full_charge_capacity_mwh: Some(66_500),
                wear_percent: Some(5.0),
                current_mode: BatteryChargeMode::Auto,
                ..Default::default()
            },
            battery_mode_writes: Vec::new(),
            lighting_writes: Vec::new(),
            display_mode: DisplayModeInfo {
                current_hz: 144,
                supported_hz: vec![60, 144, 165],
                width: 1920,
                height: 1080,
            },
            refresh_writes: Vec::new(),
            gpu_mode: GpuMode::Hybrid,
            gpu_writes: Vec::new(),
            cpu_boost: CpuBoostMode::Aggressive,
            cpu_boost_writes: Vec::new(),
            power_limits_writes: Vec::new(),
            toasts: Vec::new(),
            osds: Vec::new(),
            win_overlay: Some(WindowsPowerOverlay::Balanced),
            win_overlay_writes: Vec::new(),
            running_exes_lower: HashSet::new(),
            user_running_exes: vec!["notepad.exe".to_string()],
            battery_service_running: false,
            battery_service_cfg: None,
            fast_battery_pct: Some(82),
            event_log: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct MockRig {
    inner: Arc<Mutex<MockHardwareState>>,
}

impl MockRig {
    fn new(state: MockHardwareState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn backends(&self) -> WorkerBackends {
        WorkerBackends {
            diag: Box::new(self.clone()),
            telemetry: Box::new(self.clone()),
            thermal: Box::new(self.clone()),
            battery: Box::new(self.clone()),
            kbd: Box::new(self.clone()),
            gpu: Box::new(self.clone()),
            display: Box::new(self.clone()),
            tuning: Box::new(self.clone()),
            os: Box::new(self.clone()),
        }
    }
}

impl DiagnosticsBackend for MockRig {
    fn collect_diagnostics(&self) -> Result<DiagnosticReport> {
        Ok(self.inner.lock().unwrap().diag_report.clone())
    }
}

impl TelemetryBackend for MockRig {
    fn read_telemetry(&self) -> Result<TelemetrySnapshot> {
        Ok(self.inner.lock().unwrap().telemetry_snap.clone())
    }
}

impl WorkerTelemetryBackend for MockRig {
    fn read_telemetry_with_options(
        &self,
        _include_io_counters: bool,
        _allow_dgpu_wake: bool,
    ) -> Result<TelemetrySnapshot> {
        Ok(self.inner.lock().unwrap().telemetry_snap.clone())
    }

    fn is_pawnio_active(&self) -> bool {
        self.inner.lock().unwrap().pawnio_active
    }

    fn reload_pawnio(&self) -> bool {
        self.inner.lock().unwrap().reload_pawnio_result
    }
}

impl ThermalFanBackend for MockRig {
    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.perf_mode = mode;
        g.event_log.push(format!("perf:{mode:?}"));
        Ok(())
    }

    fn get_performance_mode(&self) -> Result<PerformanceMode> {
        Ok(self.inner.lock().unwrap().perf_mode)
    }

    fn set_fan_mode(&self, mode: FanMode, manual_speed_pct: Option<u8>) -> Result<()> {
        WorkerThermalBackend::set_fan_mode_with_cool_hint(self, mode, manual_speed_pct, false)
    }

    fn keep_alive_tick(&self) -> Result<()> {
        WorkerThermalBackend::keep_alive_tick_with_cool_hint(self, false)
    }
}

impl WorkerThermalBackend for MockRig {
    fn set_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        manual_speed_pct: Option<u8>,
        is_cool: bool,
    ) -> Result<()> {
        WorkerThermalBackend::set_dual_fan_mode_with_cool_hint(
            self,
            mode,
            manual_speed_pct,
            manual_speed_pct,
            is_cool,
        )
    }

    fn set_dual_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        cpu_speed_pct: Option<u8>,
        gpu_speed_pct: Option<u8>,
        is_cool: bool,
    ) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.fan_writes.push((mode, cpu_speed_pct, is_cool));
        g.dual_fan_writes
            .push((mode, cpu_speed_pct, gpu_speed_pct, is_cool));
        match (cpu_speed_pct, gpu_speed_pct) {
            (Some(c), Some(gp)) if c != gp => g.event_log.push(format!("fan:{mode:?}:{c}/{gp}")),
            (Some(c), _) => g.event_log.push(format!("fan:{mode:?}:{c}")),
            _ => g.event_log.push(format!("fan:{mode:?}")),
        }
        if g.fail_max_fan && mode == FanMode::Max {
            return Err(OmvixError::WmiCallFailed {
                command: 0x20008,
                command_type: 0x27,
                return_code: 5,
                message: "simulated Max fan failure".to_string(),
            });
        }
        g.fan_mode = mode;
        Ok(())
    }

    fn keep_alive_tick_with_cool_hint(&self, _is_cool: bool) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.keep_alive_count += 1;
        g.event_log.push("keepalive".to_string());
        Ok(())
    }
}

impl BatteryLimitBackend for MockRig {
    fn get_battery_status(&self) -> Result<BatteryStatus> {
        Ok(self.inner.lock().unwrap().battery_status.clone())
    }

    fn set_charge_mode(&self, mode: BatteryChargeMode) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.battery_status.current_mode = mode;
        g.battery_mode_writes.push(mode);
        g.event_log.push(format!("battery:{mode:?}"));
        Ok(())
    }
}

impl KeyboardBackend for MockRig {
    fn detect_keyboard_type(&self) -> Result<KeyboardType> {
        Ok(KeyboardType::FourZoneRgb)
    }

    fn set_backlight(&self, enabled: bool, brightness_pct: u8, zones: &[RgbColor]) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let mut arr = [RgbColor::default(); 4];
        for (dst, src) in arr.iter_mut().zip(zones.iter()) {
            *dst = *src;
        }
        g.lighting_writes.push((enabled, brightness_pct, arr));
        g.event_log
            .push(format!("lighting:{enabled}:{brightness_pct}"));
        Ok(())
    }
}

impl GpuSwitchBackend for MockRig {
    fn get_gpu_mode(&self) -> Result<GpuMode> {
        Ok(self.inner.lock().unwrap().gpu_mode)
    }

    fn set_gpu_mode(&self, mode: GpuMode) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.gpu_mode = mode;
        g.gpu_writes.push(mode);
        g.event_log.push(format!("gpu:{mode:?}"));
        Ok(())
    }
}

impl DisplayBackend for MockRig {
    fn get_display_mode(&self) -> Result<DisplayModeInfo> {
        Ok(self.inner.lock().unwrap().display_mode.clone())
    }

    fn set_refresh_rate(&self, hz: u32) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.display_mode.current_hz = hz;
        g.refresh_writes.push(hz);
        g.event_log.push(format!("refresh:{hz}"));
        Ok(())
    }
}

impl CpuTuningBackend for MockRig {
    fn set_cpu_boost_mode(&self, mode: CpuBoostMode) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.cpu_boost = mode;
        g.cpu_boost_writes.push(mode);
        g.event_log.push(format!("boost:{mode:?}"));
        Ok(())
    }

    fn set_power_limits(&self, pl1_watts: Option<u16>, pl2_watts: Option<u16>) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.power_limits_writes.push((pl1_watts, pl2_watts));
        g.event_log
            .push(format!("limits:{pl1_watts:?}:{pl2_watts:?}"));
        Ok(())
    }

    fn set_tcc_temp_limit(&self, _temp_c: u8) -> Result<()> {
        Ok(())
    }

    fn set_core_undervolt_mv(&self, _offset_mv: i16) -> Result<()> {
        Ok(())
    }
}

impl WorkerTuningBackend for MockRig {
    fn read_cpu_boost_mode(&self) -> Result<CpuBoostMode> {
        Ok(self.inner.lock().unwrap().cpu_boost)
    }
}

impl WorkerOsHooks for MockRig {
    fn send_toast(&self, title: &str, body: &str, severity: WorkerToastSeverity) {
        self.inner
            .lock()
            .unwrap()
            .toasts
            .push((title.to_string(), body.to_string(), severity));
    }

    fn show_osd(&self, title: &str, value: &str, perf_mode: PerformanceMode) {
        self.inner
            .lock()
            .unwrap()
            .osds
            .push((title.to_string(), value.to_string(), perf_mode));
    }

    fn read_win_power_overlay(&self) -> Option<WindowsPowerOverlay> {
        self.inner.lock().unwrap().win_overlay
    }

    fn set_win_power_overlay(&self, overlay: WindowsPowerOverlay) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.win_overlay = Some(overlay);
        g.win_overlay_writes.push(overlay);
        g.event_log.push(format!("win_power:{overlay:?}"));
        Ok(())
    }

    fn running_process_names_lower(&self) -> HashSet<String> {
        self.inner.lock().unwrap().running_exes_lower.clone()
    }

    fn list_user_running_exes(&self) -> Vec<String> {
        self.inner.lock().unwrap().user_running_exes.clone()
    }

    fn is_battery_service_running(&self) -> bool {
        self.inner.lock().unwrap().battery_service_running
    }

    fn read_battery_service_config(&self) -> Option<BatteryConfig> {
        self.inner.lock().unwrap().battery_service_cfg.clone()
    }

    fn sync_battery_service_config(&self, cfg: &BatteryConfig) -> Result<()> {
        self.inner.lock().unwrap().battery_service_cfg = Some(cfg.clone());
        Ok(())
    }

    fn read_fast_battery_pct(&self) -> Option<u8> {
        self.inner.lock().unwrap().fast_battery_pct
    }
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn create(label: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let id = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omvix_worker_it_{label}_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_test_engine(
    rig: &MockRig,
    cfg: AppConfig,
    board_id: &str,
    check_board_gate: bool,
    safe_mode: Option<SafeModeReason>,
    temp_dir: &Path,
) -> (WorkerEngine, Arc<Mutex<SharedState>>) {
    let crash_guard_path = temp_dir.join("crash_guard.json");
    let battery_history_path = temp_dir.join("battery_history.json");
    let crash_guard = Arc::new(Mutex::new(CrashGuardState::default()));
    let initial_state = SharedState {
        safe_mode,
        board_id: board_id.to_string(),
        board_known: lookup_board(board_id).is_some(),
        bios_writes_allowed: if check_board_gate {
            is_bios_write_allowed(board_id, cfg.general.experimental_board_id.as_deref())
        } else {
            true
        },
        effective_performance_mode: cfg.thermal_fan.performance_mode,
        effective_fan_mode: cfg.thermal_fan.fan_mode,
        effective_gpu_fan_mode: if cfg.thermal_fan.link_cpu_gpu_fans {
            cfg.thermal_fan.fan_mode
        } else {
            cfg.thermal_fan.gpu_fan_mode
        },
        ..SharedState::default()
    };
    let state = Arc::new(Mutex::new(initial_state));
    let engine = WorkerEngine::new(
        rig.backends(),
        cfg,
        Arc::clone(&state),
        board_id.to_string(),
        check_board_gate,
        safe_mode,
        crash_guard,
        crash_guard_path,
        BatteryHealthHistory::default(),
        battery_history_path,
    );
    (engine, state)
}

fn wait_until<F>(timeout_ms: u64, mut pred: F)
where
    F: FnMut() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if pred() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("condition timed out after {timeout_ms}ms");
}

#[test]
fn integration_safe_mode_blocks_all_hw_writes_until_exited() {
    let dir = TempDirGuard::create("safe_mode");
    let rig = MockRig::new(MockHardwareState::default());
    let (engine, state) = make_test_engine(
        &rig,
        AppConfig::default(),
        "8C99",
        true,
        Some(SafeModeReason::CliFlag),
        dir.path(),
    );
    let visible = Arc::new(AtomicBool::new(true));
    let worker = spawn_test_worker(engine, visible);

    worker.send(WorkerCommand::SetPerformanceMode(
        PerformanceMode::Performance,
    ));
    worker.send(WorkerCommand::SetFanMode {
        mode: FanMode::Max,
        gpu_mode: Some(FanMode::Max),
        manual_speed_pct: None,
        gpu_manual_speed_pct: None,
    });
    worker.send(WorkerCommand::ApplyBatteryNow);
    worker.send(WorkerCommand::ApplyLighting);
    worker.send(WorkerCommand::SetGpuMode(GpuMode::Discrete));
    worker.send(WorkerCommand::SetCpuBoost(CpuBoostMode::Disabled));
    worker.send(WorkerCommand::SetPowerLimits {
        pl1: Some(45),
        pl2: Some(90),
    });
    worker.send(WorkerCommand::RefreshRunningProcesses);

    wait_until(2000, || {
        !state.lock().unwrap().running_exes_snapshot.is_empty()
    });
    assert!(rig.inner.lock().unwrap().event_log.is_empty());

    worker.send(WorkerCommand::ExitSafeMode);
    worker.send(WorkerCommand::SetPerformanceMode(
        PerformanceMode::Performance,
    ));
    worker.send(WorkerCommand::SetFanMode {
        mode: FanMode::Manual,
        gpu_mode: Some(FanMode::Manual),
        manual_speed_pct: Some(70),
        gpu_manual_speed_pct: Some(55),
    });

    wait_until(2000, || {
        let g = rig.inner.lock().unwrap();
        g.perf_mode == PerformanceMode::Performance
            && g.fan_mode == FanMode::Manual
            && state.lock().unwrap().safe_mode.is_none()
    });
    worker.shutdown();
}

#[test]
fn integration_unknown_board_blocks_bios_writes_until_experimental_approved() {
    let dir = TempDirGuard::create("unknown_board");
    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) =
        make_test_engine(&rig, AppConfig::default(), "ZZZZ", true, None, dir.path());
    assert!(!state.lock().unwrap().bios_writes_allowed);

    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    engine.handle_command(
        WorkerCommand::SetPerformanceMode(PerformanceMode::Performance),
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Max,
            gpu_mode: Some(FanMode::Max),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(WorkerCommand::ApplyBatteryNow, &tx, &None, &visible);
    assert!(rig.inner.lock().unwrap().event_log.is_empty());

    // Non-BIOS control (OS CPU Boost power plan) still executes in read-only BIOS mode
    engine.handle_command(
        WorkerCommand::SetCpuBoost(CpuBoostMode::Enabled),
        &tx,
        &None,
        &visible,
    );
    assert_eq!(rig.inner.lock().unwrap().cpu_boost, CpuBoostMode::Enabled);

    // Approve Experimental Mode for board "ZZZZ"
    let mut cfg = AppConfig::default();
    cfg.general.experimental_board_id = Some("ZZZZ".to_string());
    engine.handle_command(WorkerCommand::UpdateConfig(cfg), &tx, &None, &visible);
    assert!(state.lock().unwrap().bios_writes_allowed);

    engine.handle_command(
        WorkerCommand::SetPerformanceMode(PerformanceMode::Quiet),
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Max,
            gpu_mode: Some(FanMode::Max),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(WorkerCommand::ApplyBatteryNow, &tx, &None, &visible);

    let g = rig.inner.lock().unwrap();
    assert_eq!(g.perf_mode, PerformanceMode::Quiet);
    assert_eq!(g.fan_mode, FanMode::Max);
    assert_eq!(g.battery_mode_writes.len(), 1);
}

#[test]
fn integration_thermal_guard_sustains_engages_max_retries_and_falls_back_to_auto() {
    let dir = TempDirGuard::create("thermal_guard");
    let hot_state = MockHardwareState {
        fail_max_fan: true,
        telemetry_snap: TelemetrySnapshot {
            cpu_temp_c: Some(96.0),
            gpu_temp_c: Some(72.0),
            chassis_temp_c: Some(42.0),
            fan_rpms: vec![2400, 2400],
            fan_max_rpms: vec![5200, 5200],
            ac_connected: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let rig = MockRig::new(hot_state);
    let (mut engine, state) =
        make_test_engine(&rig, AppConfig::default(), "8C99", true, None, dir.path());

    // Sustain 96C for default 10s -> engages Max attempt 1, then retries at 12.1s & 14.2s -> Auto fallback
    for t in [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.1, 14.2] {
        engine.poll_telemetry_step(t, true, true, false);
    }

    {
        let g = rig.inner.lock().unwrap();
        let max_attempts = g
            .fan_writes
            .iter()
            .filter(|(m, _, _)| *m == FanMode::Max)
            .count();
        assert_eq!(max_attempts, 3);
        assert!(matches!(
            g.fan_writes.last(),
            Some((FanMode::Auto, None, false))
        ));
        assert!(g
            .toasts
            .iter()
            .any(|(_, _, sev)| *sev == WorkerToastSeverity::Error));
        assert!(state.lock().unwrap().thermal_guard_active);
    }

    // Normal guard engage with physical RPM acceleration, followed by cool release back to CustomCurve
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.fan_mode = FanMode::CustomCurve;
    cfg.thermal_fan.gpu_fan_mode = FanMode::CustomCurve;
    let healthy_hot = MockHardwareState {
        fail_max_fan: false,
        telemetry_snap: TelemetrySnapshot {
            cpu_temp_c: Some(96.0),
            gpu_temp_c: Some(70.0),
            chassis_temp_c: Some(41.0),
            fan_rpms: vec![2400, 2400],
            fan_max_rpms: vec![5200, 5200],
            ac_connected: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let rig_ok = MockRig::new(healthy_hot);
    let (mut engine_ok, state_ok) = make_test_engine(&rig_ok, cfg, "8C99", true, None, dir.path());

    for t in [0.0, 3.0, 6.0, 9.0, 10.0] {
        engine_ok.poll_telemetry_step(t, true, true, false);
    }
    assert_eq!(rig_ok.inner.lock().unwrap().fan_mode, FanMode::Max);

    // Simulate Max fan accelerating physically + cooling below release thresholds across 6 polls
    {
        let mut g = rig_ok.inner.lock().unwrap();
        g.telemetry_snap.fan_rpms = vec![4800, 4800];
        g.telemetry_snap.cpu_temp_c = Some(74.0);
        g.telemetry_snap.gpu_temp_c = Some(62.0);
        g.telemetry_snap.chassis_temp_c = Some(38.0);
    }
    for t in [11.0, 12.0, 13.0, 14.0, 15.0, 16.0] {
        engine_ok.poll_telemetry_step(t, true, true, false);
    }
    assert!(!state_ok.lock().unwrap().thermal_guard_active);
    assert_eq!(rig_ok.inner.lock().unwrap().fan_mode, FanMode::CustomCurve);
}

#[test]
fn integration_sensor_loss_engages_auto_pauses_keepalive_and_restores_curve() {
    let dir = TempDirGuard::create("sensor_loss");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.fan_mode = FanMode::CustomCurve;
    cfg.thermal_fan.gpu_fan_mode = FanMode::CustomCurve;
    cfg.thermal_fan.keep_alive_enabled = true;

    let mut mock = MockHardwareState {
        fan_mode: FanMode::CustomCurve,
        ..Default::default()
    };
    mock.telemetry_snap.cpu_temp_c = None;
    mock.telemetry_snap.gpu_temp_c = None;
    mock.telemetry_snap.chassis_temp_c = None;

    let rig = MockRig::new(mock);
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());

    for t in [0.0, 2.0, 5.0] {
        engine.poll_telemetry_step(t, true, false, false);
    }
    assert!(!state.lock().unwrap().sensor_loss_active);
    assert!(engine.poll_keep_alive_step());
    assert_eq!(rig.inner.lock().unwrap().keep_alive_count, 1);

    // At 6s without any temperature reading -> forces BIOS Auto & pauses keep-alive ticks
    engine.poll_telemetry_step(6.0, true, false, false);
    assert!(state.lock().unwrap().sensor_loss_active);
    assert_eq!(rig.inner.lock().unwrap().fan_mode, FanMode::Auto);
    assert!(!engine.poll_keep_alive_step());
    assert_eq!(rig.inner.lock().unwrap().keep_alive_count, 1);
    assert!(rig
        .inner
        .lock()
        .unwrap()
        .toasts
        .iter()
        .any(|(_, _, sev)| *sev == WorkerToastSeverity::Warning));

    // Temperature returns -> restores CustomCurve & resumes keep-alive
    rig.inner.lock().unwrap().telemetry_snap.cpu_temp_c = Some(68.0);
    engine.poll_telemetry_step(8.0, true, false, false);
    assert!(!state.lock().unwrap().sensor_loss_active);
    assert_eq!(rig.inner.lock().unwrap().fan_mode, FanMode::CustomCurve);
    assert!(engine.poll_keep_alive_step());
    assert_eq!(rig.inner.lock().unwrap().keep_alive_count, 2);
}

#[test]
fn integration_change_history_and_undo_reverts_hardware_and_syncs_config() {
    let dir = TempDirGuard::create("undo_history");
    let rig = MockRig::new(MockHardwareState::default());
    let mut cfg = AppConfig::default();
    cfg.tuning.cpu_boost_mode = Some(CpuBoostMode::Aggressive);
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    engine.handle_command(
        WorkerCommand::SetPerformanceMode(PerformanceMode::Performance),
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: Some(FanMode::Manual),
            manual_speed_pct: Some(75),
            gpu_manual_speed_pct: Some(55),
        },
        &tx,
        &None,
        &visible,
    );

    let mut cfg_bat = engine.cfg.clone();
    cfg_bat.battery.enabled = true;
    cfg_bat.battery.mode = BatteryChargeMode::Inhibit;
    cfg_bat.battery.stop_threshold_pct = 80;
    engine.handle_command(WorkerCommand::UpdateConfig(cfg_bat), &tx, &None, &visible);
    engine.handle_command(WorkerCommand::ApplyBatteryNow, &tx, &None, &visible);

    engine.handle_command(
        WorkerCommand::SetCpuBoost(CpuBoostMode::Disabled),
        &tx,
        &None,
        &visible,
    );

    let mut cfg_light = engine.cfg.clone();
    cfg_light.lighting.enabled = true;
    cfg_light.lighting.brightness_pct = 40;
    engine.handle_command(WorkerCommand::UpdateConfig(cfg_light), &tx, &None, &visible);
    engine.handle_command(WorkerCommand::ApplyLighting, &tx, &None, &visible);

    let perf_entry_id = {
        let st = state.lock().unwrap();
        assert_eq!(st.change_history.len(), 5);
        st.change_history
            .iter()
            .find(|e| e.domain == ChangeDomain::PerformanceMode)
            .expect("performance entry")
            .id
    };

    // Undo latest (KeyboardLighting -> back to default 100% / enabled)
    engine.handle_command(WorkerCommand::UndoLastChange, &tx, &None, &visible);
    {
        let st = state.lock().unwrap();
        assert_eq!(st.change_history.len(), 4);
        assert_eq!(st.synced_config_rev, 1);
        assert_eq!(
            st.synced_config.as_ref().unwrap().lighting.brightness_pct,
            LightingConfig::default().brightness_pct
        );
    }

    // Undo specific PerformanceMode entry by id
    engine.handle_command(
        WorkerCommand::UndoChangeById(perf_entry_id),
        &tx,
        &None,
        &visible,
    );
    {
        let st = state.lock().unwrap();
        assert_eq!(st.change_history.len(), 3);
        assert_eq!(st.synced_config_rev, 2);
        assert_eq!(
            st.synced_config
                .as_ref()
                .unwrap()
                .thermal_fan
                .performance_mode,
            PerformanceMode::Balanced
        );
    }
    assert_eq!(
        rig.inner.lock().unwrap().perf_mode,
        PerformanceMode::Balanced
    );
}

#[test]
fn integration_ac_dc_auto_profile_and_app_profile_restore_baseline() {
    let dir = TempDirGuard::create("profiles");
    let mut cfg = AppConfig::default();
    cfg.auto_profile.enabled = true;
    cfg.auto_profile.ac_performance_mode = PerformanceMode::Balanced;
    cfg.auto_profile.dc_performance_mode = PerformanceMode::Quiet;
    cfg.auto_profile.dc_fan_mode = Some(FanMode::Auto);
    cfg.auto_profile.dc_refresh_hz = Some(60);

    cfg.app_profiles.enabled = true;
    cfg.app_profiles.rules.push(AppProfileRule {
        id: "cp2077".to_string(),
        name: "Cyberpunk".to_string(),
        exe_name: "cyberpunk2077.exe".to_string(),
        enabled: true,
        performance_mode: PerformanceMode::Performance,
        fan_mode: Some(FanMode::CustomCurve),
        curve_preset: Some(FanCurvePreset::Aggressive),
        refresh_hz: Some(165),
        cpu_boost_mode: Some(CpuBoostMode::Aggressive),
    });

    let mut mock = MockHardwareState::default();
    mock.telemetry_snap.ac_connected = Some(false);
    let rig = MockRig::new(mock);
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());

    // Transition to DC triggers AC/DC auto-profile (Quiet + Auto + 60Hz)
    engine.poll_telemetry_step(1.0, true, false, false);
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(g.perf_mode, PerformanceMode::Quiet);
        assert_eq!(g.fan_mode, FanMode::Auto);
        assert_eq!(g.display_mode.current_hz, 60);
        assert_eq!(state.lock().unwrap().auto_applied_revision, 1);
    }

    // Launch matching app -> activates app profile overrides
    rig.inner
        .lock()
        .unwrap()
        .running_exes_lower
        .insert("cyberpunk2077.exe".to_string());
    engine.poll_app_profiles_step();
    {
        let g = rig.inner.lock().unwrap();
        let st = state.lock().unwrap();
        assert_eq!(st.active_app_profile.as_deref(), Some("Cyberpunk"));
        assert_eq!(g.perf_mode, PerformanceMode::Performance);
        assert_eq!(g.fan_mode, FanMode::CustomCurve);
        assert_eq!(g.display_mode.current_hz, 165);
        assert_eq!(g.cpu_boost, CpuBoostMode::Aggressive);
    }

    // App closes -> first miss debounces, second miss restores DC baseline
    rig.inner.lock().unwrap().running_exes_lower.clear();
    engine.poll_app_profiles_step();
    assert_eq!(
        state.lock().unwrap().active_app_profile.as_deref(),
        Some("Cyberpunk")
    );

    engine.poll_app_profiles_step();
    {
        let g = rig.inner.lock().unwrap();
        let st = state.lock().unwrap();
        assert!(st.active_app_profile.is_none());
        assert_eq!(g.perf_mode, PerformanceMode::Quiet);
        assert_eq!(g.fan_mode, FanMode::Auto);
        assert_eq!(g.display_mode.current_hz, 60);
    }
}

#[test]
fn integration_battery_care_threshold_toast_and_off_state_safety_order() {
    let dir = TempDirGuard::create("battery_offstate");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.fan_mode = FanMode::Manual;
    cfg.battery.enabled = true;
    cfg.battery.mode = BatteryChargeMode::Inhibit;
    cfg.battery.stop_threshold_pct = 80;
    cfg.battery.start_threshold_pct = 75;
    cfg.battery.hold_when_shutdown = true;

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, _state) = make_test_engine(&rig, cfg.clone(), "8C99", true, None, dir.path());

    // First poll above stop threshold (82%) engages Inhibit & emits limit toast once
    assert!(engine.poll_battery_step());
    assert!(engine.poll_battery_step());
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(g.battery_mode_writes, vec![BatteryChargeMode::Inhibit]);
        assert_eq!(g.toasts.len(), 1);
        assert_eq!(g.toasts[0].2, WorkerToastSeverity::Info);
    }

    // SystemSuspending must flush Fan Auto strictly before Battery Hold and release ack cleanly
    rig.inner.lock().unwrap().event_log.clear();
    let visible = Arc::new(AtomicBool::new(true));
    let worker = spawn_test_worker(engine, visible);
    let (ack_tx, ack_rx) = mpsc::channel();
    worker.send(WorkerCommand::SystemSuspending(ack_tx));
    ack_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("suspend ack");
    {
        let g = rig.inner.lock().unwrap();
        let fan_idx = g
            .event_log
            .iter()
            .position(|e| e == "fan:Auto")
            .expect("fan:Auto logged");
        let bat_idx = g
            .event_log
            .iter()
            .position(|e| e == "battery:Hold")
            .expect("battery:Hold logged");
        assert!(fan_idx < bat_idx);
    }
    worker.shutdown();

    // Clean Shutdown with hold_when_shutdown=false restores Auto fan before Auto battery
    cfg.battery.hold_when_shutdown = false;
    let rig2 = MockRig::new(MockHardwareState::default());
    let (engine2, _st2) = make_test_engine(&rig2, cfg, "8C99", true, None, dir.path());
    let worker2 = spawn_test_worker(engine2, Arc::new(AtomicBool::new(true)));
    worker2.shutdown();
    let g2 = rig2.inner.lock().unwrap();
    let fan_idx = g2
        .event_log
        .iter()
        .position(|e| e == "fan:Auto")
        .expect("fan:Auto logged");
    let bat_idx = g2
        .event_log
        .iter()
        .position(|e| e == "battery:Auto")
        .expect("battery:Auto logged");
    assert!(fan_idx < bat_idx);
}

#[test]
fn integration_telemetry_csv_export_and_battery_health_history_logging() {
    let dir = TempDirGuard::create("csv_bat_hist");
    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) =
        make_test_engine(&rig, AppConfig::default(), "8C99", true, None, dir.path());

    engine.poll_telemetry_step(1.0, true, false, false);
    rig.inner.lock().unwrap().telemetry_snap.cpu_temp_c = Some(58.0);
    engine.poll_telemetry_step(2.0, true, false, false);

    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));
    let csv_path = dir.path().join("telemetry.csv");

    engine.handle_command(
        WorkerCommand::ExportTelemetryCsv(csv_path.clone()),
        &tx,
        &None,
        &visible,
    );
    engine.handle_command(WorkerCommand::RecordBatteryHealthNow, &tx, &None, &visible);

    let csv_text = fs::read_to_string(&csv_path).expect("csv saved");
    assert_eq!(csv_text.lines().count(), 3);
    let st = state.lock().unwrap();
    assert_eq!(st.last_csv_export_path.as_ref(), Some(&csv_path));
    assert_eq!(st.battery_health_history.len(), 1);
    assert!(dir.path().join("battery_history.json").is_file());
}

#[test]
fn integration_service_cleared_travel_charge_syncs_and_stays_cleared() {
    let dir = TempDirGuard::create("svc_travel_sync");
    let mut cfg = AppConfig::default();
    cfg.battery.enabled = true;
    cfg.battery.mode = BatteryChargeMode::Inhibit;
    cfg.battery.enable_travel_charge(Utc::now().timestamp());

    // Simulate background service having completed 1x travel charge and persisted cleared flag
    let mut cleared_svc_bat = cfg.battery.clone();
    cleared_svc_bat.clear_travel_charge();
    let mock = MockHardwareState {
        battery_service_running: true,
        battery_service_cfg: Some(cleared_svc_bat),
        ..Default::default()
    };

    let rig = MockRig::new(mock);
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());

    assert!(engine.poll_battery_step());
    assert!(!engine.cfg.battery.one_time_full_charge);
    assert!(engine.cfg.battery.travel_charge_until_utc.is_none());

    let st = state.lock().unwrap();
    assert_eq!(st.synced_config_rev, 1);
    let synced = st.synced_config.as_ref().expect("synced config published");
    assert!(!synced.battery.one_time_full_charge);
    assert!(synced.battery.travel_charge_until_utc.is_none());
}

#[test]
fn integration_apply_battery_now_never_skips_when_firmware_mode_unknown() {
    let dir = TempDirGuard::create("bat_unknown_mode");
    let mut cfg = AppConfig::default();
    cfg.battery.enabled = true;
    cfg.battery.mode = BatteryChargeMode::Inhibit;
    cfg.battery.stop_threshold_pct = 80;
    cfg.battery.start_threshold_pct = 75;

    let mut mock = MockHardwareState::default();
    mock.battery_status.charge_percent = 85;
    mock.battery_status.ac_connected = true;
    mock.battery_status.current_mode = BatteryChargeMode::Unknown;

    let rig = MockRig::new(mock);
    let (mut engine, _state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    engine.last_applied_bat_mode = Some(BatteryChargeMode::Inhibit);

    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));
    assert!(!engine.handle_command(WorkerCommand::ApplyBatteryNow, &tx, &None, &visible));

    let g = rig.inner.lock().unwrap();
    assert_eq!(g.battery_mode_writes, vec![BatteryChargeMode::Inhibit]);
}

#[test]
fn integration_restore_default_power_limits_reapplies_profile_and_keeps_other_channel() {
    let dir = TempDirGuard::create("power_limit_reset");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.performance_mode = PerformanceMode::Performance;
    cfg.tuning.pl1_watts = Some(55);
    cfg.tuning.pl2_watts = Some(115);

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, _state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    // Clear only PL1 while keeping PL2 -> must re-apply active profile first, then apply (None, Some(95))
    engine.handle_command(
        WorkerCommand::RestoreDefaultPowerLimits {
            keep_pl1: None,
            keep_pl2: Some(95),
        },
        &tx,
        &None,
        &visible,
    );
    assert_eq!(engine.cfg.tuning.pl1_watts, None);
    assert_eq!(engine.cfg.tuning.pl2_watts, Some(95));
    assert_eq!(engine.last_committed_power_limits, (None, Some(95)));
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(
            g.event_log,
            vec![
                "perf:Performance".to_string(),
                "win_power:BestPerformance".to_string(),
                "limits:None:Some(95)".to_string(),
            ]
        );
    }

    // Clear all power limits -> re-applies active profile ONLY without calling set_power_limits
    rig.inner.lock().unwrap().event_log.clear();
    rig.inner.lock().unwrap().power_limits_writes.clear();
    engine.handle_command(
        WorkerCommand::RestoreDefaultPowerLimits {
            keep_pl1: None,
            keep_pl2: None,
        },
        &tx,
        &None,
        &visible,
    );
    assert_eq!(engine.cfg.tuning.pl1_watts, None);
    assert_eq!(engine.cfg.tuning.pl2_watts, None);
    assert_eq!(engine.last_committed_power_limits, (None, None));
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(
            g.event_log,
            vec![
                "perf:Performance".to_string(),
                "win_power:BestPerformance".to_string(),
            ]
        );
        assert!(g.power_limits_writes.is_empty());
    }
}

#[test]
fn integration_thermal_guard_release_resolves_custom_curve_target() {
    let dir = TempDirGuard::create("guard_release_curve");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.fan_mode = FanMode::CustomCurve;
    cfg.thermal_fan.gpu_fan_mode = FanMode::CustomCurve;
    cfg.thermal_fan.manual_speed_pct = 12;
    cfg.thermal_fan.custom_curve = FanCurvePreset::Aggressive.to_curve();
    cfg.thermal_fan.guard_cpu_temp_c = 85;
    cfg.thermal_fan.guard_sustain_secs = 2;

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());

    // Sustain overheating (>= 85C) so Thermal Guard engages emergency Max Fan
    for t in [1.0, 2.5, 4.0] {
        let mut g = rig.inner.lock().unwrap();
        g.telemetry_snap.cpu_temp_c = Some(92.0);
        g.telemetry_snap.fan_rpms = vec![1400, 1400];
        drop(g);
        engine.poll_telemetry_step(t, true, true, false);
    }
    assert!(state.lock().unwrap().thermal_guard_active);

    // Cool down well below release threshold (with cool ambient <60C? Use 62C so curve speed != manual 12%)
    {
        let mut g = rig.inner.lock().unwrap();
        g.telemetry_snap.cpu_temp_c = Some(62.0);
        g.telemetry_snap.gpu_temp_c = Some(55.0);
        g.telemetry_snap.chassis_temp_c = Some(40.0);
        g.telemetry_snap.fan_rpms = vec![5000, 5000];
        g.fan_writes.clear();
    }
    engine.poll_telemetry_step(15.0, true, true, false);
    engine.poll_telemetry_step(25.0, true, true, false);

    assert!(!state.lock().unwrap().thermal_guard_active);
    let expected_curve_pct = FanCurvePreset::Aggressive
        .to_curve()
        .evaluate(62.0, 62.0, 0);
    assert_ne!(expected_curve_pct, 12);
    let writes = rig.inner.lock().unwrap().fan_writes.clone();
    assert!(writes
        .iter()
        .any(|&(mode, pct, _)| mode == FanMode::CustomCurve && pct == Some(expected_curve_pct)));
}

#[test]
fn integration_first_time_power_limits_recorded_and_undoable_to_defaults() {
    let dir = TempDirGuard::create("first_power_limit_undo");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.performance_mode = PerformanceMode::Balanced;
    cfg.tuning.pl1_watts = None;
    cfg.tuning.pl2_watts = None;

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    engine.handle_command(
        WorkerCommand::SetPowerLimits {
            pl1: Some(45),
            pl2: Some(90),
        },
        &tx,
        &None,
        &visible,
    );
    assert_eq!(engine.change_history.entries_vec().len(), 1);
    assert_eq!(engine.last_committed_power_limits, (Some(45), Some(90)));

    rig.inner.lock().unwrap().event_log.clear();
    rig.inner.lock().unwrap().power_limits_writes.clear();

    engine.handle_command(WorkerCommand::UndoLastChange, &tx, &None, &visible);
    assert!(engine.change_history.entries_vec().is_empty());
    assert!(state.lock().unwrap().change_history.is_empty());
    assert_eq!(engine.cfg.tuning.pl1_watts, None);
    assert_eq!(engine.cfg.tuning.pl2_watts, None);
    assert_eq!(engine.last_committed_power_limits, (None, None));
    assert_eq!(
        rig.inner.lock().unwrap().event_log,
        vec![
            "perf:Balanced".to_string(),
            "win_power:Balanced".to_string()
        ]
    );
}

#[test]
fn integration_blocked_and_failed_undo_preserve_config_and_history() {
    let dir = TempDirGuard::create("blocked_failed_undo");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.fan_mode = FanMode::Max;
    cfg.thermal_fan.gpu_fan_mode = FanMode::Max;
    cfg.thermal_fan.manual_speed_pct = 50;

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    // Change FanMode: Max -> Manual (65%) so undo wants to revert to Max
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: Some(FanMode::Manual),
            manual_speed_pct: Some(65),
            gpu_manual_speed_pct: Some(45),
        },
        &tx,
        &None,
        &visible,
    );
    assert_eq!(engine.change_history.entries_vec().len(), 1);
    let sync_before = state.lock().unwrap().synced_config_rev;

    // 1. Hardware failure while reverting to Max -> cfg & history stay unchanged
    rig.inner.lock().unwrap().fail_max_fan = true;
    engine.handle_command(WorkerCommand::UndoLastChange, &tx, &None, &visible);
    assert_eq!(engine.cfg.thermal_fan.fan_mode, FanMode::Manual);
    assert_eq!(engine.cfg.thermal_fan.manual_speed_pct, 65);
    assert_eq!(engine.change_history.entries_vec().len(), 1);
    assert_eq!(state.lock().unwrap().change_history.len(), 1);
    assert_eq!(state.lock().unwrap().synced_config_rev, sync_before);

    // 2. Guard block (Thermal Guard active) -> cfg & history stay unchanged even without backend error
    rig.inner.lock().unwrap().fail_max_fan = false;
    for t in [1.0, 12.0] {
        let mut g = rig.inner.lock().unwrap();
        g.telemetry_snap.cpu_temp_c = Some(92.0);
        g.telemetry_snap.fan_rpms = vec![1400, 1400];
        drop(g);
        engine.poll_telemetry_step(t, true, true, false);
    }
    assert!(engine.thermal_guard.is_active());

    engine.handle_command(WorkerCommand::UndoLastChange, &tx, &None, &visible);
    assert_eq!(engine.cfg.thermal_fan.fan_mode, FanMode::Manual);
    assert_eq!(engine.cfg.thermal_fan.manual_speed_pct, 65);
    assert_eq!(engine.change_history.entries_vec().len(), 1);
    assert_eq!(state.lock().unwrap().change_history.len(), 1);
    assert_eq!(state.lock().unwrap().synced_config_rev, sync_before);
}

#[test]
fn integration_independent_cpu_and_gpu_fans_manual_and_curves() {
    let dir = TempDirGuard::create("dual_fans");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.link_cpu_gpu_fans = false;
    cfg.thermal_fan.custom_curve = FanCurvePreset::Quiet.to_curve();
    cfg.thermal_fan.gpu_custom_curve = FanCurvePreset::Aggressive.to_curve();

    let mut mock = MockHardwareState::default();
    mock.telemetry_snap.cpu_temp_c = Some(48.0);
    mock.telemetry_snap.gpu_temp_c = Some(72.0);
    mock.telemetry_snap.chassis_temp_c = Some(39.0);
    let rig = MockRig::new(mock);
    let (mut engine, _state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    // 1. Independent manual speeds write distinct CPU & GPU percentages
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: Some(FanMode::Manual),
            manual_speed_pct: Some(40),
            gpu_manual_speed_pct: Some(75),
        },
        &tx,
        &None,
        &visible,
    );
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(
            g.dual_fan_writes.last().copied(),
            Some((FanMode::Manual, Some(40), Some(75), false))
        );
    }

    // Populate initial telemetry snapshot in SharedState (like startup phase 1),
    // then switch to CustomCurve -> evaluates CPU curve on 48C Quiet & GPU curve on 72C Aggressive
    engine.poll_telemetry_step(1.0, true, false, false);
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::CustomCurve,
            gpu_mode: Some(FanMode::CustomCurve),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );
    let expected_cpu = FanCurvePreset::Quiet.to_curve().evaluate(48.0, 48.0, 0);
    let expected_gpu = FanCurvePreset::Aggressive
        .to_curve()
        .evaluate(72.0, 72.0, 0);
    assert_ne!(expected_cpu, expected_gpu);
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(
            g.dual_fan_writes.last().copied(),
            Some((
                FanMode::CustomCurve,
                Some(expected_cpu),
                Some(expected_gpu),
                false
            ))
        );
    }

    // 3. When GPU enters sleep & temps drop, telemetry curve poll updates GPU speed independently
    {
        let mut g = rig.inner.lock().unwrap();
        g.telemetry_snap.gpu_sleeping = true;
        g.telemetry_snap.gpu_temp_c = None;
        g.telemetry_snap.chassis_temp_c = Some(36.0);
    }
    engine.poll_telemetry_step(4.0, true, false, true);
    let expected_gpu_sleep = FanCurvePreset::Aggressive
        .to_curve()
        .evaluate(36.0, 36.0, 0);
    let last = rig
        .inner
        .lock()
        .unwrap()
        .dual_fan_writes
        .last()
        .copied()
        .unwrap();
    assert_eq!(last.0, FanMode::CustomCurve);
    assert_eq!(last.1, Some(expected_cpu));
    assert_eq!(last.2, Some(expected_gpu_sleep));
}

#[test]
fn integration_independent_cpu_and_gpu_fan_modes_mixed_and_auto() {
    let dir = TempDirGuard::create("mixed_fan_modes");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.link_cpu_gpu_fans = false;
    cfg.thermal_fan.custom_curve = FanCurvePreset::Quiet.to_curve();

    let mut mock = MockHardwareState::default();
    mock.telemetry_snap.cpu_temp_c = Some(46.0);
    mock.telemetry_snap.gpu_temp_c = Some(60.0);
    mock.telemetry_snap.chassis_temp_c = Some(38.0);
    let rig = MockRig::new(mock);
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));

    // Populate telemetry first
    engine.poll_telemetry_step(1.0, true, false, false);

    // 1. CPU = Auto, GPU = Manual 75% -> dual write uses smart auto curve for CPU and 75% for GPU
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Auto,
            gpu_mode: Some(FanMode::Manual),
            manual_speed_pct: None,
            gpu_manual_speed_pct: Some(75),
        },
        &tx,
        &None,
        &visible,
    );
    let expected_auto_cpu = FanCurve::preset_ogh_like().evaluate(46.0, 46.0, 0);
    assert_ne!(expected_auto_cpu, 75);
    {
        let st = state.lock().unwrap();
        assert_eq!(st.effective_fan_mode, FanMode::Auto);
        assert_eq!(st.effective_gpu_fan_mode, FanMode::Manual);
    }
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(
            g.dual_fan_writes.last().copied(),
            Some((
                FanMode::CustomCurve,
                Some(expected_auto_cpu),
                Some(75),
                false
            ))
        );
    }

    // 2. CPU = CustomCurve, GPU = Max -> CPU from curve, GPU = 100%
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::CustomCurve,
            gpu_mode: Some(FanMode::Max),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );
    let expected_curve_cpu = FanCurvePreset::Quiet.to_curve().evaluate(46.0, 46.0, 0);
    assert_eq!(
        rig.inner.lock().unwrap().dual_fan_writes.last().copied(),
        Some((
            FanMode::CustomCurve,
            Some(expected_curve_cpu),
            Some(100),
            false
        ))
    );

    // 3. Reverting both to Auto restores native BIOS Auto (None, None)
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Auto,
            gpu_mode: Some(FanMode::Auto),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );
    assert_eq!(
        rig.inner.lock().unwrap().dual_fan_writes.last().copied(),
        Some((FanMode::Auto, None, None, false))
    );
}

#[test]
fn integration_set_fan_mode_blocked_while_thermal_guard_active() {
    let dir = TempDirGuard::create("fan_blocked_guard");
    let hot_state = MockHardwareState {
        telemetry_snap: TelemetrySnapshot {
            cpu_temp_c: Some(96.0),
            gpu_temp_c: Some(72.0),
            chassis_temp_c: Some(41.0),
            fan_rpms: vec![2400, 2400],
            fan_max_rpms: vec![5200, 5200],
            ac_connected: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let rig = MockRig::new(hot_state);
    let (mut engine, state) =
        make_test_engine(&rig, AppConfig::default(), "8C99", true, None, dir.path());

    for t in [0.0, 4.0, 8.0, 10.5] {
        engine.poll_telemetry_step(t, true, true, false);
    }
    assert!(state.lock().unwrap().thermal_guard_active);
    let writes_before = rig.inner.lock().unwrap().dual_fan_writes.len();

    let (tx, _rx) = mpsc::channel();
    let visible = Arc::new(AtomicBool::new(true));
    engine.handle_command(
        WorkerCommand::SetFanMode {
            mode: FanMode::Auto,
            gpu_mode: Some(FanMode::Auto),
            manual_speed_pct: None,
            gpu_manual_speed_pct: None,
        },
        &tx,
        &None,
        &visible,
    );

    assert_eq!(
        rig.inner.lock().unwrap().dual_fan_writes.len(),
        writes_before
    );
    assert_eq!(engine.cfg.thermal_fan.fan_mode, FanMode::Auto);
    assert!(state.lock().unwrap().thermal_guard_active);
}

#[test]
fn integration_coalesce_merges_partial_fan_mode_commands() {
    let cmds = vec![
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: None,
            manual_speed_pct: Some(65),
            gpu_manual_speed_pct: None,
        },
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: Some(FanMode::CustomCurve),
            manual_speed_pct: None,
            gpu_manual_speed_pct: Some(50),
        },
    ];
    let coalesced = coalesce_worker_commands(cmds);
    assert_eq!(coalesced.len(), 1);
    assert!(matches!(
        coalesced[0],
        WorkerCommand::SetFanMode {
            mode: FanMode::Manual,
            gpu_mode: Some(FanMode::CustomCurve),
            manual_speed_pct: Some(65),
            gpu_manual_speed_pct: Some(50),
        }
    ));
}

#[test]
fn integration_app_profile_rule_switch_restores_none_fields_and_gpu_baseline() {
    let dir = TempDirGuard::create("app_profile_switch_none");
    let mut cfg = AppConfig::default();
    cfg.thermal_fan.performance_mode = PerformanceMode::Balanced;
    cfg.thermal_fan.fan_mode = FanMode::Auto;
    cfg.thermal_fan.gpu_fan_mode = FanMode::Manual;
    cfg.thermal_fan.gpu_manual_speed_pct = 62;
    cfg.app_profiles.enabled = true;
    cfg.app_profiles.rules = vec![
        AppProfileRule {
            id: "r1".into(),
            name: "Heavy Game".into(),
            exe_name: "heavy.exe".into(),
            enabled: true,
            performance_mode: PerformanceMode::Performance,
            fan_mode: Some(FanMode::Max),
            curve_preset: None,
            refresh_hz: Some(60),
            cpu_boost_mode: Some(CpuBoostMode::Disabled),
        },
        AppProfileRule {
            id: "r2".into(),
            name: "Light App".into(),
            exe_name: "light.exe".into(),
            enabled: true,
            performance_mode: PerformanceMode::Quiet,
            fan_mode: None,
            curve_preset: None,
            refresh_hz: None,
            cpu_boost_mode: None,
        },
    ];

    let rig = MockRig::new(MockHardwareState::default());
    let (mut engine, state) = make_test_engine(&rig, cfg, "8C99", true, None, dir.path());
    {
        let mut st = state.lock().unwrap();
        st.cpu_boost = Some(CpuBoostMode::Aggressive);
        st.display_mode = Some(DisplayModeInfo {
            width: 1920,
            height: 1080,
            current_hz: 165,
            supported_hz: vec![60, 165],
        });
    }
    engine.poll_telemetry_step(1.0, true, false, false);
    let expected_auto_cpu = FanCurve::preset_ogh_like().evaluate(52.0, 45.0, 40);

    // Enter Rule 1
    rig.inner.lock().unwrap().running_exes_lower = ["heavy.exe".to_string()].into_iter().collect();
    engine.poll_app_profiles_step();
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(g.perf_mode, PerformanceMode::Performance);
        assert_eq!(g.fan_mode, FanMode::Max);
        assert_eq!(g.cpu_boost, CpuBoostMode::Disabled);
        assert_eq!(g.display_mode.current_hz, 60);
    }

    // Switch directly to Rule 2 (None for fan/hz/boost) -> must restore baseline instead of leaking Rule 1
    rig.inner.lock().unwrap().running_exes_lower = ["light.exe".to_string()].into_iter().collect();
    engine.poll_app_profiles_step();
    {
        let g = rig.inner.lock().unwrap();
        assert_eq!(g.perf_mode, PerformanceMode::Quiet);
        assert_eq!(g.cpu_boost, CpuBoostMode::Aggressive);
        assert_eq!(g.display_mode.current_hz, 165);
        assert_eq!(
            g.dual_fan_writes.last().copied(),
            Some((
                FanMode::CustomCurve,
                Some(expected_auto_cpu),
                Some(62),
                true
            ))
        );
    }
    {
        let st = state.lock().unwrap();
        assert_eq!(st.effective_fan_mode, FanMode::Auto);
        assert_eq!(st.effective_gpu_fan_mode, FanMode::Manual);
    }
}
