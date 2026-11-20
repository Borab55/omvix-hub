use crate::system_info::collect_system_identity;
use crate::wmi_bios::{HpWmiBios, OutputSize, describe_wmi_return_code};
use omvix_core::thermal::{
    DEFAULT_MAX_FAN_LEVEL, resolve_scheme_from_identity, safe_manual_wmi_level,
    thermal_profile_byte,
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
    max_fan_levels: (u8, u8),
}

/// Windows HP WMI BIOS thermal profile and fan controller.
pub struct WindowsThermalFanBackend {
    state: Mutex<ThermalFanState>,
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
                max_fan_levels: (DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL),
            }),
        }
    }

    pub fn with_scheme(scheme: ThermalProfileScheme) -> Self {
        Self {
            state: Mutex::new(ThermalFanState {
                scheme: Some(scheme),
                performance_mode: PerformanceMode::Balanced,
                fan_mode: FanMode::Auto,
                manual_speed_pct: 50,
                max_fan_levels: (DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL),
            }),
        }
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

    fn query_max_fan_levels(bios: &HpWmiBios) -> (u8, u8) {
        if let Ok(resp) = bios.execute(0x20008, 0x2F, &[0u8; 4], OutputSize::Size128) {
            if resp.return_code == 0 {
                let parsed = HpWmiBios::parse_max_fan_levels_from_table(&resp.data);
                let f1 = parsed.first().copied().unwrap_or(DEFAULT_MAX_FAN_LEVEL);
                let f2 = parsed.get(1).copied().unwrap_or(f1);
                return (f1, f2);
            }
        }
        (DEFAULT_MAX_FAN_LEVEL, DEFAULT_MAX_FAN_LEVEL)
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

    fn apply_fan_commands(
        bios: &HpWmiBios,
        mode: FanMode,
        manual_speed_pct: Option<u8>,
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
                let pct = manual_speed_pct.unwrap_or(50);
                let l1 = safe_manual_wmi_level(pct, max_levels.0);
                let l2 = safe_manual_wmi_level(pct, max_levels.1);
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
        let scheme = self.detect_scheme();
        let mode_byte = thermal_profile_byte(scheme, mode);
        let fan_mode = self
            .state
            .lock()
            .map(|g| g.fan_mode)
            .unwrap_or(FanMode::Auto);
        let by_bios = if fan_mode == FanMode::Auto { 0x01 } else { 0x00 };

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
        let scheme = self.detect_scheme();
        let perf_mode = self
            .state
            .lock()
            .map(|g| g.performance_mode)
            .unwrap_or(PerformanceMode::Balanced);

        let max_levels = HpWmiBios::with_cached(|bios| {
            let max_levels = Self::query_max_fan_levels(bios);
            Self::apply_fan_commands(bios, mode, manual_speed_pct, max_levels, scheme, perf_mode)?;
            Ok(max_levels)
        })?;

        if let Ok(mut guard) = self.state.lock() {
            guard.fan_mode = mode;
            guard.max_fan_levels = max_levels;
            if let Some(pct) = manual_speed_pct {
                guard.manual_speed_pct = pct.clamp(0, 100);
            }
        }
        tracing::info!(?mode, ?manual_speed_pct, ?max_levels, "Applied fan mode");
        Ok(())
    }

    fn keep_alive_tick(&self) -> Result<()> {
        let scheme = self.detect_scheme();
        let (fan_mode, manual_speed_pct, max_levels, perf_mode) = {
            let guard = self
                .state
                .lock()
                .map_err(|_| OmvixError::Config("Thermal mutex poisoned".to_string()))?;
            (
                guard.fan_mode,
                guard.manual_speed_pct,
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
                    max_levels,
                    scheme,
                    perf_mode,
                )
            })?;
            tracing::debug!(?fan_mode, manual_speed_pct, "Executed BIOS fan keep-alive tick");
        }
        Ok(())
    }
}
