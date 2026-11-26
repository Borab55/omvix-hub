use crate::system_info::collect_system_identity;
use crate::wmi_bios::{describe_wmi_return_code, HpWmiBios, OutputSize};
use omvix_core::thermal::{
    resolve_scheme_from_identity, smart_manual_wmi_level, thermal_profile_byte,
    DEFAULT_MAX_FAN_LEVEL,
};
use omvix_core::{
    FanMode, OmvixError, PerformanceMode, Result, ThermalFanBackend, ThermalProfileScheme,
};
use std::sync::Mutex;

struct ThermalFanState {
    scheme: Option<ThermalProfileScheme>,
    performance_mode: PerformanceMode,
    fan_mode: FanMode,
    manual_speed_pct: u8,
    gpu_manual_speed_pct: u8,
    allow_zero_rpm: bool,
    max_fan_levels: (u8, u8),
    max_fan_levels_loaded: bool,
}

/// Restores BIOS automatic fan control over a brand-new WMI connection without acquiring any cached
/// WMI slot or `WindowsThermalFanBackend` mutex (safe from panic recovery / watchdog threads).
pub fn restore_fan_auto_fresh_wmi() -> Result<()> {
    let scheme = collect_system_identity()
        .map(|id| resolve_scheme_from_identity(&id.board_id, &id.model, &id.system_family))
        .unwrap_or(ThermalProfileScheme::VictusS);
    let bios = HpWmiBios::connect()?;
    let clear_max = bios.execute(0x20008, 0x27, &[0x00, 0x00, 0x00, 0x00], OutputSize::Size0)?;
    if clear_max.return_code != 0 {
        return Err(OmvixError::WmiCallFailed {
            command: 0x20008,
            command_type: 0x27,
            return_code: clear_max.return_code,
            message: describe_wmi_return_code(clear_max.return_code),
        });
    }
    let mode_byte = thermal_profile_byte(scheme, PerformanceMode::Balanced);
    let apply_auto = bios.execute(
        0x20008,
        0x1A,
        &[0xFF, mode_byte, 0x01, 0x00],
        OutputSize::Size0,
    )?;
    if apply_auto.return_code != 0 {
        return Err(OmvixError::WmiCallFailed {
            command: 0x20008,
            command_type: 0x1A,
            return_code: apply_auto.return_code,
            message: describe_wmi_return_code(apply_auto.return_code),
        });
    }
    Ok(())
}

/// Windows HP WMI BIOS thermal profile and fan controller.
pub struct WindowsThermalFanBackend {
    state: Mutex<ThermalFanState>,
    wmi_op_lock: Mutex<()>,
}

impl Default for WindowsThermalFanBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsThermalFanBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ThermalFanState {
                scheme: None,
                performance_mode: PerformanceMode::Balanced,
                fan_mode: FanMode::Auto,
                manual_speed_pct: 50,
                gpu_manual_speed_pct: 50,
                allow_zero_rpm: false,
                max_fan_levels: (DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL),
                max_fan_levels_loaded: false,
            }),
            wmi_op_lock: Mutex::new(()),
        }
    }

    /// Sets the fan mode using identical target speeds for CPU and GPU fans.
    pub fn set_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        manual_speed_pct: Option<u8>,
        allow_zero_rpm: bool,
    ) -> Result<()> {
        self.set_dual_fan_mode_with_cool_hint(
            mode,
            manual_speed_pct,
            manual_speed_pct,
            allow_zero_rpm,
        )
    }

    /// Sets the fan mode with independent CPU (`0x2E[0]`) and GPU (`0x2E[1]`) fan speed targets
    /// while passing whether current thermal conditions are cool enough for sub-1800 / 0 RPM speeds.
    pub fn set_dual_fan_mode_with_cool_hint(
        &self,
        mode: FanMode,
        cpu_speed_pct: Option<u8>,
        gpu_speed_pct: Option<u8>,
        allow_zero_rpm: bool,
    ) -> Result<()> {
        let _wmi_op = self
            .wmi_op_lock
            .lock()
            .map_err(|_| OmvixError::Config("Thermal WMI operation mutex poisoned".to_string()))?;
        let scheme = self.detect_scheme();
        let (perf_mode, fallback_cpu, fallback_gpu, cached_max_levels, max_loaded) = self
            .state
            .lock()
            .map(|g| {
                (
                    g.performance_mode,
                    g.manual_speed_pct,
                    g.gpu_manual_speed_pct,
                    g.max_fan_levels,
                    g.max_fan_levels_loaded,
                )
            })
            .unwrap_or((
                PerformanceMode::Balanced,
                50,
                50,
                (DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL),
                false,
            ));
        let eff_cpu = cpu_speed_pct.or(Some(fallback_cpu));
        let eff_gpu = gpu_speed_pct.or(Some(fallback_gpu));

        let (max_levels, loaded_now) = HpWmiBios::with_cached(|bios| {
            let (max_levels, loaded_now) = if max_loaded {
                (cached_max_levels, true)
            } else {
                Self::query_max_fan_levels(bios)
            };
            Self::apply_fan_commands(
                bios,
                mode,
                eff_cpu,
                eff_gpu,
                allow_zero_rpm,
                max_levels,
                scheme,
                perf_mode,
            )?;
            Ok((max_levels, loaded_now))
        })?;

        if let Ok(mut guard) = self.state.lock() {
            guard.fan_mode = mode;
            guard.max_fan_levels = max_levels;
            guard.max_fan_levels_loaded = loaded_now;
            guard.allow_zero_rpm = allow_zero_rpm;
            if let Some(pct) = cpu_speed_pct {
                guard.manual_speed_pct = pct.clamp(0, 100);
            }
            if let Some(pct) = gpu_speed_pct {
                guard.gpu_manual_speed_pct = pct.clamp(0, 100);
            }
        }
        tracing::info!(
            ?mode,
            ?cpu_speed_pct,
            ?gpu_speed_pct,
            allow_zero_rpm,
            ?max_levels,
            "Applied fan mode"
        );
        Ok(())
    }

    /// Refreshes the HP BIOS 120-second user-defined fan state timer with the latest cool hint.
    pub fn keep_alive_tick_with_cool_hint(&self, allow_zero_rpm: bool) -> Result<()> {
        let _wmi_op = self
            .wmi_op_lock
            .lock()
            .map_err(|_| OmvixError::Config("Thermal WMI operation mutex poisoned".to_string()))?;
        let scheme = self.detect_scheme();
        let (fan_mode, manual_speed_pct, gpu_manual_speed_pct, max_levels, perf_mode) = {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| OmvixError::Config("Thermal mutex poisoned".to_string()))?;
            guard.allow_zero_rpm = allow_zero_rpm;
            (
                guard.fan_mode,
                guard.manual_speed_pct,
                guard.gpu_manual_speed_pct,
                guard.max_fan_levels,
                guard.performance_mode,
            )
        };

        if matches!(
            fan_mode,
            FanMode::Manual | FanMode::CustomCurve | FanMode::Max
        ) {
            HpWmiBios::with_cached(|bios| {
                Self::apply_fan_commands(
                    bios,
                    fan_mode,
                    Some(manual_speed_pct),
                    Some(gpu_manual_speed_pct),
                    allow_zero_rpm,
                    max_levels,
                    scheme,
                    perf_mode,
                )
            })?;
            tracing::debug!(
                ?fan_mode,
                manual_speed_pct,
                gpu_manual_speed_pct,
                allow_zero_rpm,
                "Executed BIOS fan keep-alive tick"
            );
        }
        Ok(())
    }

    fn detect_scheme(&self) -> ThermalProfileScheme {
        if let Ok(guard) = self.state.lock() {
            if let Some(s) = guard.scheme {
                return s;
            }
        }
        let detected = collect_system_identity()
            .map(|id| resolve_scheme_from_identity(&id.board_id, &id.model, &id.system_family))
            .unwrap_or(ThermalProfileScheme::VictusS);
        if let Ok(mut guard) = self.state.lock() {
            guard.scheme = Some(detected);
        }
        detected
    }

    fn query_max_fan_levels(bios: &HpWmiBios) -> ((u8, u8), bool) {
        if let Ok(resp) = bios.execute(0x20008, 0x2F, &[0u8; 4], OutputSize::Size128) {
            if resp.return_code == 0 {
                let parsed = HpWmiBios::parse_max_fan_levels_from_table(&resp.data);
                let f1 = parsed.first().copied().unwrap_or(DEFAULT_MAX_FAN_LEVEL);
                let f2 = parsed.get(1).copied().unwrap_or(f1);
                return ((f1, f2), !parsed.is_empty());
            }
        }
        ((DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL), false)
    }

    fn write_bios_checked(
        bios: &HpWmiBios,
        command: u32,
        command_type: u32,
        payload: &[u8],
    ) -> Result<()> {
        let resp = bios.execute(command, command_type, payload, OutputSize::Size0)?;
        if resp.return_code == 0 {
            Ok(())
        } else {
            Err(OmvixError::WmiCallFailed {
                command,
                command_type,
                return_code: resp.return_code,
                message: describe_wmi_return_code(resp.return_code),
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_fan_commands(
        bios: &HpWmiBios,
        mode: FanMode,
        cpu_speed_pct: Option<u8>,
        gpu_speed_pct: Option<u8>,
        allow_zero_rpm: bool,
        max_levels: (u8, u8),
        scheme: ThermalProfileScheme,
        performance_mode: PerformanceMode,
    ) -> Result<()> {
        match mode {
            FanMode::Auto => {
                // CRITICAL: NEVER write `0x2E [0, 0]` for Auto mode! On Victus 8C99 / OMEN firmware,
                // level 0 means "fans OFF (0 RPM)" until user-defined state expires (~120s).
                // Clear Max Fan (0x27 [0]) and re-apply 0x1A with `byBios = 0x01` to restore BIOS control.
                Self::write_bios_checked(bios, 0x20008, 0x27, &[0x00, 0x00, 0x00, 0x00])?;
                let mode_byte = thermal_profile_byte(scheme, performance_mode);
                Self::write_bios_checked(bios, 0x20008, 0x1A, &[0xFF, mode_byte, 0x01, 0x00])?;
            }
            FanMode::Max => {
                let _ = bios.execute(0x20008, 0x10, &[0u8; 4], OutputSize::Size4);
                Self::write_bios_checked(bios, 0x20008, 0x27, &[0x01, 0x00, 0x00, 0x00])?;
            }
            FanMode::Manual | FanMode::CustomCurve => {
                let cpu_pct = cpu_speed_pct.unwrap_or(50);
                let gpu_pct = gpu_speed_pct.unwrap_or(cpu_pct);
                let l1 = smart_manual_wmi_level(cpu_pct, max_levels.0, allow_zero_rpm);
                let l2 = smart_manual_wmi_level(gpu_pct, max_levels.1, allow_zero_rpm);
                let _ = bios.execute(0x20008, 0x10, &[0u8; 4], OutputSize::Size4);
                Self::write_bios_checked(bios, 0x20008, 0x27, &[0x00, 0x00, 0x00, 0x00])?;
                Self::write_bios_checked(bios, 0x20008, 0x2E, &[l1, l2, 0x00, 0x00])?;
            }
        }
        Ok(())
    }
}

impl ThermalFanBackend for WindowsThermalFanBackend {
    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<()> {
        let _wmi_op = self
            .wmi_op_lock
            .lock()
            .map_err(|_| OmvixError::Config("Thermal WMI operation mutex poisoned".to_string()))?;
        let scheme = self.detect_scheme();
        let mode_byte = thermal_profile_byte(scheme, mode);
        let fan_mode = self
            .state
            .lock()
            .map(|g| g.fan_mode)
            .unwrap_or(FanMode::Auto);
        let by_bios = if fan_mode == FanMode::Auto {
            0x01
        } else {
            0x00
        };

        HpWmiBios::with_cached(|bios| {
            Self::write_bios_checked(bios, 0x20008, 0x1A, &[0xFF, mode_byte, by_bios, 0x00])
        })?;

        if let Ok(mut guard) = self.state.lock() {
            guard.performance_mode = mode;
        }
        tracing::info!(
            ?mode,
            ?scheme,
            by_bios,
            byte = %format!("0x{mode_byte:02X}"),
            "Applied performance mode"
        );
        Ok(())
    }

    fn get_performance_mode(&self) -> Result<PerformanceMode> {
        let guard = self
            .state
            .lock()
            .map_err(|_| OmvixError::Config("Thermal mutex poisoned".to_string()))?;
        Ok(guard.performance_mode)
    }

    fn set_fan_mode(&self, mode: FanMode, manual_speed_pct: Option<u8>) -> Result<()> {
        self.set_fan_mode_with_cool_hint(mode, manual_speed_pct, false)
    }

    fn keep_alive_tick(&self) -> Result<()> {
        let allow_zero = self.state.lock().map(|g| g.allow_zero_rpm).unwrap_or(false);
        self.keep_alive_tick_with_cool_hint(allow_zero)
    }
}
