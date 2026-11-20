use crate::wmi_bios::{HpWmiBios, OutputSize};
use crate::wmi_com::{WmiConnection, get_u32};
use chrono::Utc;
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use omvix_core::{Result, TelemetryBackend, TelemetrySnapshot};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

struct TelemetryState {
    sys: System,
    last_cpu_refresh: Instant,
    nvml: Option<Nvml>,
    nvml_init_attempted: bool,
}

/// Windows telemetry provider combining `sysinfo`, NVIDIA NVML, Win32 Power API, and optional WMI sensors.
pub struct WindowsTelemetryBackend {
    state: Mutex<TelemetryState>,
}

impl Default for WindowsTelemetryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsTelemetryBackend {
    pub fn new() -> Self {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());
        let mut sys = System::new_with_specifics(refresh);
        sys.refresh_cpu_all();
        sys.refresh_memory();

        Self {
            state: Mutex::new(TelemetryState {
                sys,
                last_cpu_refresh: Instant::now(),
                nvml: None,
                nvml_init_attempted: false,
            }),
        }
    }

    fn read_acpi_cpu_temp_c() -> Option<f32> {
        let wmi = WmiConnection::connect(r"root\WMI").ok()?;
        let rows = wmi
            .query("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
            .ok()?;
        let mut max_temp: Option<f32> = None;
        for row in rows {
            if let Some(tenths_kelvin) = get_u32(&row, "CurrentTemperature") {
                let celsius = (tenths_kelvin as f32 / 10.0) - 273.15;
                if (15.0..=115.0).contains(&celsius) {
                    max_temp = Some(max_temp.map_or(celsius, |m| m.max(celsius)));
                }
            }
        }
        max_temp
    }

    fn read_win32_power() -> (Option<bool>, Option<u8>, Option<bool>) {
        let mut status = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return (None, None, None);
        }
        let ac = match status.ACLineStatus {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        let pct = if status.BatteryLifePercent <= 100 {
            Some(status.BatteryLifePercent)
        } else {
            None
        };
        let charging = if status.BatteryFlag == 255 || (status.BatteryFlag & 128) != 0 {
            None
        } else {
            Some((status.BatteryFlag & 8) != 0)
        };
        (ac, pct, charging)
    }

    fn read_wmi_battery_rate_w() -> Option<f32> {
        let wmi = WmiConnection::connect(r"root\WMI").ok()?;
        let rows = wmi
            .query("SELECT Charging, Discharging, ChargeRate, DischargeRate FROM BatteryStatus")
            .ok()?;
        let row = rows.into_iter().next()?;
        let charge_mw = get_u32(&row, "ChargeRate").unwrap_or(0);
        let discharge_mw = get_u32(&row, "DischargeRate").unwrap_or(0);
        if charge_mw > 0 && charge_mw < 500_000 {
            Some(charge_mw as f32 / 1000.0)
        } else if discharge_mw > 0 && discharge_mw < 500_000 {
            Some(-(discharge_mw as f32 / 1000.0))
        } else {
            Some(0.0)
        }
    }

    fn read_wmi_fans_and_temp() -> (Vec<u32>, Option<f32>) {
        let Ok(bios) = HpWmiBios::connect() else {
            return (Vec::new(), None);
        };

        let fan_count = bios
            .execute(0x20008, 0x10, &[0u8; 4], OutputSize::Size4)
            .ok()
            .filter(|r| r.return_code == 0 && !r.data.is_empty())
            .map(|r| (r.data[0] as usize).clamp(0, 4))
            .unwrap_or(2);

        let mut rpms = Vec::new();
        if fan_count > 0 {
            if let Ok(resp) = bios.execute(0x20008, 0x2D, &[0u8; 4], OutputSize::Size128) {
                if resp.return_code == 0 {
                    for &raw in resp.data.iter().take(fan_count) {
                        // Victus-S / Omen 0x2D typically reports fan speed in hundreds of RPM.
                        rpms.push(u32::from(raw) * 100);
                    }
                }
            }
        }

        let temp_c = bios
            .execute(0x20008, 0x23, &[0u8; 4], OutputSize::Size4)
            .ok()
            .filter(|r| r.return_code == 0 && !r.data.is_empty())
            .map(|r| r.data[0] as f32)
            .filter(|t| (15.0..=115.0).contains(t));

        (rpms, temp_c)
    }
}

impl TelemetryBackend for WindowsTelemetryBackend {
    fn read_telemetry(&self) -> Result<TelemetrySnapshot> {
        let mut guard = self.state.lock().map_err(|_| {
            omvix_core::OmvixError::Config("Telemetry mutex poisoned".to_string())
        })?;

        let elapsed = guard.last_cpu_refresh.elapsed();
        if elapsed < Duration::from_millis(220) {
            std::thread::sleep(Duration::from_millis(220) - elapsed);
        }

        guard.sys.refresh_cpu_all();
        guard.sys.refresh_memory();
        guard.last_cpu_refresh = Instant::now();

        let cpu_usage_pct = Some(guard.sys.global_cpu_usage());
        let cpu_clock_mhz = guard.sys.cpus().first().map(|c| c.frequency() as u32);
        let ram_total_gb = Some(guard.sys.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0));
        let ram_used_gb = Some(guard.sys.used_memory() as f32 / (1024.0 * 1024.0 * 1024.0));

        if !guard.nvml_init_attempted {
            guard.nvml_init_attempted = true;
            match Nvml::init() {
                Ok(nvml) => {
                    tracing::info!("Initialized NVIDIA NVML telemetry");
                    guard.nvml = Some(nvml);
                }
                Err(err) => {
                    tracing::debug!("NVML initialization skipped/unavailable: {err}");
                }
            }
        }

        let mut gpu_name = None;
        let mut gpu_temp_c = None;
        let mut gpu_usage_pct = None;
        let mut gpu_power_w = None;
        let mut gpu_clock_mhz = None;

        if let Some(nvml) = &guard.nvml {
            if let Ok(device) = nvml.device_by_index(0) {
                gpu_name = device.name().ok();
                gpu_temp_c = device.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32);
                gpu_usage_pct = device.utilization_rates().ok().map(|u| u.gpu as f32);
                gpu_power_w = device.power_usage().ok().map(|mw| mw as f32 / 1000.0);
                gpu_clock_mhz = device.clock_info(Clock::Graphics).ok();
            }
        }

        drop(guard);

        let (ac_connected, battery_pct, _charging) = Self::read_win32_power();
        let battery_rate_w = Self::read_wmi_battery_rate_w();
        let (fan_rpms, wmi_temp_c) = Self::read_wmi_fans_and_temp();
        let cpu_temp_c = wmi_temp_c.or_else(Self::read_acpi_cpu_temp_c);

        Ok(TelemetrySnapshot {
            timestamp: Some(Utc::now()),
            cpu_temp_c,
            cpu_usage_pct,
            cpu_clock_mhz,
            gpu_name,
            gpu_temp_c,
            gpu_usage_pct,
            gpu_power_w,
            gpu_clock_mhz,
            ram_used_gb,
            ram_total_gb,
            fan_rpms,
            ac_connected,
            battery_pct,
            battery_rate_w,
        })
    }
}
