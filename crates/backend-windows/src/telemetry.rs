use crate::emi::CpuEmiReader;
use crate::pawnio::PawnIoCpuTempReader;
use crate::wmi_bios::{HpWmiBios, OutputSize};
use crate::wmi_com::{WmiConnection, get_u32};
use chrono::Utc;
use nvml_wrapper::Nvml;
use nvml_wrapper::bitmasks::device::ThrottleReasons;
use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor};
use omvix_core::thermal::DEFAULT_MAX_FAN_LEVEL;
use omvix_core::{Result, TelemetryBackend, TelemetrySnapshot};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

const EC_POLL_INTERVAL: Duration = Duration::from_millis(1000);

struct TelemetryState {
    sys: System,
    networks: Networks,
    disks: Disks,
    nvml: Option<Nvml>,
    nvml_init_attempted: bool,
    emi: Option<CpuEmiReader>,
    pawnio: Option<PawnIoCpuTempReader>,
    first_sample_done: bool,
    last_fast_refresh: Instant,
    last_disk_list_refresh: Instant,
    last_ec_poll: Option<Instant>,
    fan_count: Option<usize>,
    fan_max_rpms: Vec<u32>,
    cached_fan_rpms: Vec<u32>,
    cached_chassis_temp_c: Option<f32>,
    cached_battery_rate_w: Option<f32>,
    cached_battery_voltage_v: Option<f32>,
    cached_battery_remaining_mwh: Option<u32>,
}

/// Dual-tier Windows telemetry provider:
/// - **Fast tier (supports 100 ms / 0.1 s polling):** `sysinfo` CPU/RAM/Disk/Net, NVML GPU metrics,
///   Windows Energy Meter Interface (`EMI`) CPU package power, and `GetSystemPowerStatus`.
/// - **EC / WMI tier (capped at >= 1000 ms):** Persistent `HpWmiBios` fan RPM / CPU thermal
///   sensor and `root\WMI` `BatteryStatus` wattage/voltage/capacity.
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

        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list_specifics(
            DiskRefreshKind::nothing().with_io_usage(),
        );
        let emi = CpuEmiReader::open();
        let pawnio = PawnIoCpuTempReader::open();

        Self {
            state: Mutex::new(TelemetryState {
                sys,
                networks,
                disks,
                nvml: None,
                nvml_init_attempted: false,
                emi,
                pawnio,
                first_sample_done: false,
                last_fast_refresh: Instant::now(),
                last_disk_list_refresh: Instant::now(),
                last_ec_poll: None,
                fan_count: None,
                fan_max_rpms: Vec::new(),
                cached_fan_rpms: Vec::new(),
                cached_chassis_temp_c: None,
                cached_battery_rate_w: None,
                cached_battery_voltage_v: None,
                cached_battery_remaining_mwh: None,
            }),
        }
    }

    pub fn reload_pawnio(&self) -> bool {
        let reader = PawnIoCpuTempReader::open();
        let active = reader.is_some();
        if let Ok(mut guard) = self.state.lock() {
            guard.pawnio = reader;
        }
        active
    }

    pub fn is_pawnio_active(&self) -> bool {
        self.state
            .lock()
            .map(|g| g.pawnio.is_some())
            .unwrap_or(false)
    }

    fn read_win32_power() -> (Option<bool>, Option<u8>, Option<u32>) {
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
        let lifetime_min = if status.BatteryLifeTime != u32::MAX && status.BatteryLifeTime > 0 {
            Some(status.BatteryLifeTime / 60)
        } else {
            None
        };
        (ac, pct, lifetime_min)
    }

    fn poll_ec_and_wmi_tier(
        cached_fan_count: Option<usize>,
        cached_fan_max_rpms: &[u32],
    ) -> (
        Option<usize>,
        Vec<u32>,
        Vec<u32>,
        Option<f32>,
        Option<f32>,
        Option<f32>,
        Option<u32>,
    ) {
        let mut fan_count = cached_fan_count;
        let mut fan_max_rpms = cached_fan_max_rpms.to_vec();
        let mut fan_rpms = Vec::new();
        let mut chassis_temp_c = None;

        if let Ok((fc, max_rpms, rpms, temp)) = HpWmiBios::with_cached(|bios| {
            // Read fan count & table from 0x2F (NOT 0x10, because 0x10 arms HP firmware's
            // 120-second user-defined fan timer on Victus S / OMEN boards!).
            let (count, resolved_max) = if let (Some(c), false) =
                (cached_fan_count, cached_fan_max_rpms.is_empty())
            {
                (c, cached_fan_max_rpms.to_vec())
            } else if let Ok(tbl) = bios.execute(0x20008, 0x2F, &[0u8; 4], OutputSize::Size128) {
                if tbl.return_code == 0 {
                    let parsed_count = tbl
                        .data
                        .first()
                        .copied()
                        .map(|b| (b as usize).clamp(1, 4))
                        .unwrap_or(2);
                    let max_rpms: Vec<u32> = HpWmiBios::parse_max_fan_levels_from_table(&tbl.data)
                        .into_iter()
                        .map(|lvl| u32::from(lvl) * 100)
                        .collect();
                    (parsed_count, max_rpms)
                } else {
                    let c = cached_fan_count.unwrap_or(2);
                    (c, vec![u32::from(DEFAULT_MAX_FAN_LEVEL) * 100; c])
                }
            } else {
                let c = cached_fan_count.unwrap_or(2);
                (c, vec![u32::from(DEFAULT_MAX_FAN_LEVEL) * 100; c])
            };

            let mut current_rpms = Vec::new();
            if count > 0 {
                if let Ok(resp) = bios.execute(0x20008, 0x2D, &[0u8; 4], OutputSize::Size128) {
                    if resp.return_code == 0 {
                        for &raw in resp.data.iter().take(count) {
                            current_rpms.push(u32::from(raw) * 100);
                        }
                    }
                }
            }

            // EC 0x23 reports the board Chassis / IR thermal sensor, NOT CPU die temp.
            let ch_temp = bios
                .execute(0x20008, 0x23, &[0u8; 4], OutputSize::Size4)
                .ok()
                .filter(|r| r.return_code == 0 && !r.data.is_empty())
                .map(|r| r.data[0] as f32)
                .filter(|t| (15.0..=115.0).contains(t));

            Ok((count, resolved_max, current_rpms, ch_temp))
        }) {
            fan_count = Some(fc);
            fan_max_rpms = max_rpms;
            fan_rpms = rpms;
            chassis_temp_c = temp;
        }

        let mut battery_rate_w = None;
        let mut battery_voltage_v = None;
        let mut battery_remaining_mwh = None;

        let _ = WmiConnection::with_cached_root_wmi(|wmi| {
            if let Ok(rows) = wmi.query(
                "SELECT ChargeRate, DischargeRate, Voltage, RemainingCapacity FROM BatteryStatus",
            ) {
                if let Some(row) = rows.into_iter().next() {
                    let charge_mw = get_u32(&row, "ChargeRate").unwrap_or(0);
                    let discharge_mw = get_u32(&row, "DischargeRate").unwrap_or(0);
                    battery_rate_w = if charge_mw > 0 && charge_mw < 500_000 {
                        Some(charge_mw as f32 / 1000.0)
                    } else if discharge_mw > 0 && discharge_mw < 500_000 {
                        Some(-(discharge_mw as f32 / 1000.0))
                    } else {
                        Some(0.0)
                    };
                    battery_voltage_v = get_u32(&row, "Voltage")
                        .filter(|&mv| mv > 0 && mv < 50_000)
                        .map(|mv| mv as f32 / 1000.0);
                    battery_remaining_mwh =
                        get_u32(&row, "RemainingCapacity").filter(|&c| c > 0 && c < 1_000_000);
                }
            }

            Ok(())
        });

        (
            fan_count,
            fan_max_rpms,
            fan_rpms,
            chassis_temp_c,
            battery_rate_w,
            battery_voltage_v,
            battery_remaining_mwh,
        )
    }
}

impl TelemetryBackend for WindowsTelemetryBackend {
    fn read_telemetry(&self) -> Result<TelemetrySnapshot> {
        let (
            cpu_usage_pct,
            cpu_temp_c,
            cpu_clock_mhz,
            cpu_power_w,
            ram_used_gb,
            ram_total_gb,
            gpu_name,
            gpu_temp_c,
            gpu_usage_pct,
            gpu_power_w,
            gpu_power_limit_w,
            gpu_clock_mhz,
            gpu_mem_clock_mhz,
            gpu_mem_used_mb,
            gpu_mem_total_mb,
            gpu_pstate,
            gpu_throttle_reason,
            disk_read_mb_s,
            disk_write_mb_s,
            net_rx_mb_s,
            net_tx_mb_s,
            need_ec_poll,
            cached_fc,
            cached_max_rpms,
            cached_rpms,
            cached_chassis_temp,
            cached_bat_rate,
            cached_bat_v,
            cached_bat_rem,
        ) = {
            let mut guard = self.state.lock().map_err(|_| {
                omvix_core::OmvixError::Config("Telemetry mutex poisoned".to_string())
            })?;

            // Wait briefly on only the very first sample so CLI 1-shot commands collect a real
            // CPU usage & EMI power window; subsequent 0.1s polls have zero artificial delay.
            if !guard.first_sample_done {
                let elapsed = guard.last_fast_refresh.elapsed();
                let min_first = Duration::from_millis(75);
                if elapsed < min_first {
                    std::thread::sleep(min_first - elapsed);
                }
                guard.first_sample_done = true;
            }

            let dt_secs = guard.last_fast_refresh.elapsed().as_secs_f32().max(0.05);
            guard.sys.refresh_cpu_all();
            guard.sys.refresh_memory();
            guard.networks.refresh(false);

            if guard.last_disk_list_refresh.elapsed() >= Duration::from_secs(15) {
                guard
                    .disks
                    .refresh_specifics(true, DiskRefreshKind::nothing().with_io_usage());
                guard.last_disk_list_refresh = Instant::now();
            } else {
                for d in guard.disks.list_mut() {
                    d.refresh_specifics(DiskRefreshKind::nothing().with_io_usage());
                }
            }
            guard.last_fast_refresh = Instant::now();

            let cpu_usage_pct = Some(guard.sys.global_cpu_usage());
            let cpu_clock_mhz = guard.sys.cpus().first().map(|c| c.frequency() as u32);
            let ram_total_gb =
                Some(guard.sys.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0));
            let ram_used_gb = Some(guard.sys.used_memory() as f32 / (1024.0 * 1024.0 * 1024.0));

            let cpu_power_w = guard.emi.as_mut().and_then(|e| e.sample_cpu_power_w());
            let cpu_temp_c = guard.pawnio.as_mut().and_then(|p| p.read_package_temp_c());

            let mut total_rx = 0u64;
            let mut total_tx = 0u64;
            for (iface, data) in &guard.networks {
                if iface.contains("Loopback") {
                    continue;
                }
                total_rx = total_rx.saturating_add(data.received());
                total_tx = total_tx.saturating_add(data.transmitted());
            }
            let net_rx_mb_s = Some((total_rx as f32 / (1024.0 * 1024.0)) / dt_secs);
            let net_tx_mb_s = Some((total_tx as f32 / (1024.0 * 1024.0)) / dt_secs);

            let mut total_disk_r = 0u64;
            let mut total_disk_w = 0u64;
            for disk in guard.disks.list() {
                let usage = disk.usage();
                total_disk_r = total_disk_r.saturating_add(usage.read_bytes);
                total_disk_w = total_disk_w.saturating_add(usage.written_bytes);
            }
            let disk_read_mb_s = Some((total_disk_r as f32 / (1024.0 * 1024.0)) / dt_secs);
            let disk_write_mb_s = Some((total_disk_w as f32 / (1024.0 * 1024.0)) / dt_secs);

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
            let mut gpu_power_limit_w = None;
            let mut gpu_clock_mhz = None;
            let mut gpu_mem_clock_mhz = None;
            let mut gpu_mem_used_mb = None;
            let mut gpu_mem_total_mb = None;
            let mut gpu_pstate = None;
            let mut gpu_throttle_reason = None;

            if let Some(nvml) = &guard.nvml {
                if let Ok(device) = nvml.device_by_index(0) {
                    gpu_name = device.name().ok();
                    gpu_temp_c = device.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32);
                    gpu_usage_pct = device.utilization_rates().ok().map(|u| u.gpu as f32);
                    gpu_power_w = device.power_usage().ok().map(|mw| mw as f32 / 1000.0);
                    gpu_power_limit_w = device
                        .enforced_power_limit()
                        .or_else(|_| device.power_management_limit())
                        .ok()
                        .map(|mw| mw as f32 / 1000.0);
                    gpu_clock_mhz = device.clock_info(Clock::Graphics).ok();
                    gpu_mem_clock_mhz = device.clock_info(Clock::Memory).ok();
                    if let Ok(mem) = device.memory_info() {
                        gpu_mem_used_mb = Some((mem.used / (1024 * 1024)) as u32);
                        gpu_mem_total_mb = Some((mem.total / (1024 * 1024)) as u32);
                    }
                    gpu_pstate = device.performance_state().ok().map(format_pstate);
                    gpu_throttle_reason = device
                        .current_throttle_reasons()
                        .ok()
                        .and_then(format_throttle_reasons);
                }
            }

            let need_ec_poll = guard
                .last_ec_poll
                .is_none_or(|t| t.elapsed() >= EC_POLL_INTERVAL);

            (
                cpu_usage_pct,
                cpu_temp_c,
                cpu_clock_mhz,
                cpu_power_w,
                ram_used_gb,
                ram_total_gb,
                gpu_name,
                gpu_temp_c,
                gpu_usage_pct,
                gpu_power_w,
                gpu_power_limit_w,
                gpu_clock_mhz,
                gpu_mem_clock_mhz,
                gpu_mem_used_mb,
                gpu_mem_total_mb,
                gpu_pstate,
                gpu_throttle_reason,
                disk_read_mb_s,
                disk_write_mb_s,
                net_rx_mb_s,
                net_tx_mb_s,
                need_ec_poll,
                guard.fan_count,
                guard.fan_max_rpms.clone(),
                guard.cached_fan_rpms.clone(),
                guard.cached_chassis_temp_c,
                guard.cached_battery_rate_w,
                guard.cached_battery_voltage_v,
                guard.cached_battery_remaining_mwh,
            )
        };

        let (ac_connected, battery_pct, win_lifetime_min) = Self::read_win32_power();

        let (
            fan_rpms,
            fan_max_rpms,
            chassis_temp_c,
            battery_rate_w,
            battery_voltage_v,
            battery_remaining_mwh,
        ) = if need_ec_poll {
            let (fc, max_rpms, rpms, ch_temp_c, rate_w, volt_v, rem_mwh) =
                Self::poll_ec_and_wmi_tier(cached_fc, &cached_max_rpms);

            if let Ok(mut guard) = self.state.lock() {
                guard.last_ec_poll = Some(Instant::now());
                if fc.is_some() {
                    guard.fan_count = fc;
                }
                if !max_rpms.is_empty() {
                    guard.fan_max_rpms = max_rpms.clone();
                }
                if !rpms.is_empty() {
                    guard.cached_fan_rpms = rpms.clone();
                }
                if ch_temp_c.is_some() {
                    guard.cached_chassis_temp_c = ch_temp_c;
                }
                if rate_w.is_some() {
                    guard.cached_battery_rate_w = rate_w;
                }
                if volt_v.is_some() {
                    guard.cached_battery_voltage_v = volt_v;
                }
                if rem_mwh.is_some() {
                    guard.cached_battery_remaining_mwh = rem_mwh;
                }
            }

            (
                if rpms.is_empty() { cached_rpms } else { rpms },
                if max_rpms.is_empty() {
                    cached_max_rpms
                } else {
                    max_rpms
                },
                ch_temp_c.or(cached_chassis_temp),
                rate_w.or(cached_bat_rate),
                volt_v.or(cached_bat_v),
                rem_mwh.or(cached_bat_rem),
            )
        } else {
            (
                cached_rpms,
                cached_max_rpms,
                cached_chassis_temp,
                cached_bat_rate,
                cached_bat_v,
                cached_bat_rem,
            )
        };

        let battery_time_remaining_min = win_lifetime_min.or_else(|| {
            if ac_connected == Some(false) {
                match (battery_remaining_mwh, battery_rate_w) {
                    (Some(rem_mwh), Some(rate_w)) if rate_w < -0.5 => {
                        let hrs = (rem_mwh as f32 / 1000.0) / (-rate_w);
                        Some((hrs * 60.0).round().clamp(1.0, 2880.0) as u32)
                    }
                    _ => None,
                }
            } else {
                None
            }
        });

        Ok(TelemetrySnapshot {
            timestamp: Some(Utc::now()),
            cpu_temp_c,
            chassis_temp_c,
            cpu_usage_pct,
            cpu_clock_mhz,
            cpu_power_w,
            gpu_name,
            gpu_temp_c,
            gpu_usage_pct,
            gpu_power_w,
            gpu_power_limit_w,
            gpu_clock_mhz,
            gpu_mem_clock_mhz,
            gpu_mem_used_mb,
            gpu_mem_total_mb,
            gpu_pstate,
            gpu_throttle_reason,
            ram_used_gb,
            ram_total_gb,
            fan_rpms,
            fan_max_rpms,
            ac_connected,
            battery_pct,
            battery_rate_w,
            battery_voltage_v,
            battery_remaining_mwh,
            battery_time_remaining_min,
            disk_read_mb_s,
            disk_write_mb_s,
            net_rx_mb_s,
            net_tx_mb_s,
        })
    }
}

fn format_pstate(p: PerformanceState) -> String {
    match p {
        PerformanceState::Zero => "P0".to_string(),
        PerformanceState::One => "P1".to_string(),
        PerformanceState::Two => "P2".to_string(),
        PerformanceState::Three => "P3".to_string(),
        PerformanceState::Four => "P4".to_string(),
        PerformanceState::Five => "P5".to_string(),
        PerformanceState::Six => "P6".to_string(),
        PerformanceState::Seven => "P7".to_string(),
        PerformanceState::Eight => "P8".to_string(),
        PerformanceState::Nine => "P9".to_string(),
        PerformanceState::Ten => "P10".to_string(),
        PerformanceState::Eleven => "P11".to_string(),
        PerformanceState::Twelve => "P12".to_string(),
        PerformanceState::Thirteen => "P13".to_string(),
        PerformanceState::Fourteen => "P14".to_string(),
        PerformanceState::Fifteen => "P15".to_string(),
        PerformanceState::Unknown => "Unknown".to_string(),
    }
}

fn format_throttle_reasons(reasons: ThrottleReasons) -> Option<String> {
    let mut active = Vec::new();
    if reasons.contains(ThrottleReasons::HW_THERMAL_SLOWDOWN)
        || reasons.contains(ThrottleReasons::SW_THERMAL_SLOWDOWN)
    {
        active.push("Thermal");
    }
    if reasons.contains(ThrottleReasons::SW_POWER_CAP)
        || reasons.contains(ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN)
    {
        active.push("Power");
    }
    if reasons.contains(ThrottleReasons::HW_SLOWDOWN) {
        active.push("HW Slowdown");
    }
    if active.is_empty() {
        None
    } else {
        Some(active.join(" + "))
    }
}
