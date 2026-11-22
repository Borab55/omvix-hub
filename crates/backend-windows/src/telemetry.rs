use crate::emi::CpuEmiReader;
use crate::pawnio::PawnIoCpuTempReader;
use crate::wmi_bios::{HpWmiBios, OutputSize};
use crate::wmi_com::{get_u32, WmiConnection};
use chrono::Utc;
use nvml_wrapper::bitmasks::device::ThrottleReasons;
use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor};
use nvml_wrapper::Nvml;
use omvix_core::thermal::DEFAULT_MAX_FAN_LEVEL;
use omvix_core::{Result, TelemetryBackend, TelemetrySnapshot};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY,
    SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_PowerData, DEVPROPTYPE};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

const EC_POLL_INTERVAL: Duration = Duration::from_millis(1000);

struct TelemetryState {
    sys: System,
    networks: Networks,
    disks: Disks,
    nvml: Option<Nvml>,
    nvml_last_attempt: Option<Instant>,
    nvml_attempts: u8,
    last_nvml_poll: Option<Instant>,
    gpu_smoother: GpuSampleSmoother,
    emi: Option<CpuEmiReader>,
    pawnio: Option<PawnIoCpuTempReader>,
    pawnio_last_attempt: Option<Instant>,
    pawnio_attempts: u8,
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

/// Returns `true` when lazy driver/SDK initialization (NVML or PawnIO) should be attempted or retried.
pub fn should_retry_init(
    attempts: u8,
    max_attempts: u8,
    last_attempt: Option<Instant>,
    cooldown: Duration,
) -> bool {
    attempts < max_attempts && last_attempt.is_none_or(|t| t.elapsed() >= cooldown)
}

/// Filters implausible NVML GPU board power readings emitted during Optimus D3cold transitions.
pub fn sanitize_nvml_power_w(raw_w: f32, power_limit_w: Option<f32>) -> Option<f32> {
    if !raw_w.is_finite() || raw_w < 0.0 {
        return None;
    }
    let ceiling = power_limit_w
        .filter(|l| l.is_finite() && *l > 0.0)
        .map(|l| (l * 1.6).max(220.0))
        .unwrap_or(300.0);
    (raw_w <= ceiling).then_some(raw_w)
}

/// Validates NVML GPU core temperature readings.
pub fn sanitize_nvml_temp_c(raw_c: f32) -> Option<f32> {
    (raw_c.is_finite() && (5.0..=115.0).contains(&raw_c)).then_some(raw_c)
}

/// Validates NVML GPU utilization readings.
pub fn sanitize_nvml_usage_pct(raw_pct: u32) -> Option<f32> {
    (raw_pct <= 100).then_some(raw_pct as f32)
}

/// Windows PnP device power state for an NVIDIA discrete GPU (`DEVPKEY_Device_PowerData`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPowerState {
    AwakeD0,
    SleepingD3,
    Unknown,
}

/// Decodes `CM_POWER_DATA.PD_MostRecentPowerState` without touching NVML or waking a D3 GPU.
pub fn parse_cm_power_data_state(buf: &[u8]) -> GpuPowerState {
    if buf.len() < 8 {
        return GpuPowerState::Unknown;
    }
    let pd_size = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if pd_size < 8 {
        return GpuPowerState::Unknown;
    }
    let state = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    match state {
        1 => GpuPowerState::AwakeD0,
        2..=4 => GpuPowerState::SleepingD3,
        _ => GpuPowerState::Unknown,
    }
}

/// Queries the NVIDIA display adapter's PnP D-state via SetupAPI (`DEVPKEY_Device_PowerData`).
/// Reading PnP metadata from Windows device manager state never wakes a GPU in RTD3/D3cold sleep.
pub fn query_nvidia_pnp_power_state() -> GpuPowerState {
    let Ok(hdev) =
        (unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), None, None, DIGCF_PRESENT) })
    else {
        return GpuPowerState::Unknown;
    };

    let mut found_sleeping = false;
    for idx in 0..32u32 {
        let mut dev_info = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(hdev, idx, &mut dev_info) }.is_err() {
            break;
        }
        let mut id_buf = [0u16; 256];
        if unsafe { SetupDiGetDeviceInstanceIdW(hdev, &dev_info, Some(&mut id_buf), None) }.is_err()
        {
            continue;
        }
        let len = id_buf.iter().position(|&c| c == 0).unwrap_or(id_buf.len());
        let instance_id = String::from_utf16_lossy(&id_buf[..len]).to_ascii_uppercase();
        if !instance_id.contains("PCI\\VEN_10DE") {
            continue;
        }
        let mut prop_type = DEVPROPTYPE::default();
        let mut data_buf = [0u8; 64];
        let mut req_size = 0u32;
        if unsafe {
            SetupDiGetDevicePropertyW(
                hdev,
                &dev_info,
                &DEVPKEY_Device_PowerData,
                &mut prop_type,
                Some(&mut data_buf),
                Some(&mut req_size),
                0,
            )
        }
        .is_ok()
        {
            let used = (req_size as usize).min(data_buf.len());
            match parse_cm_power_data_state(&data_buf[..used]) {
                GpuPowerState::AwakeD0 => {
                    unsafe {
                        let _ = SetupDiDestroyDeviceInfoList(hdev);
                    }
                    return GpuPowerState::AwakeD0;
                }
                GpuPowerState::SleepingD3 => {
                    found_sleeping = true;
                }
                GpuPowerState::Unknown => {}
            }
        }
    }

    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(hdev);
    }
    if found_sleeping {
        GpuPowerState::SleepingD3
    } else {
        GpuPowerState::Unknown
    }
}

/// Filters NVML throttle bitmasks so thermal throttling always surfaces while transient idle P-state
/// power-cap/slowdown noise during Optimus transitions is suppressed. Returns `(reason, is_thermal)`.
pub fn filter_throttle_reasons(
    reasons: ThrottleReasons,
    gpu_usage_pct: Option<f32>,
    gpu_power_w: Option<f32>,
    gpu_temp_c: Option<f32>,
) -> (Option<String>, bool) {
    let mut active = Vec::new();
    let is_thermal = reasons.contains(ThrottleReasons::HW_THERMAL_SLOWDOWN)
        || reasons.contains(ThrottleReasons::SW_THERMAL_SLOWDOWN);
    if is_thermal {
        active.push("Thermal");
    }

    let under_load = gpu_usage_pct.unwrap_or(0.0) >= 10.0 || gpu_power_w.unwrap_or(0.0) >= 25.0;
    if under_load
        && (reasons.contains(ThrottleReasons::SW_POWER_CAP)
            || reasons.contains(ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN))
    {
        active.push("Power");
    }
    if (under_load || gpu_temp_c.unwrap_or(0.0) >= 75.0)
        && reasons.contains(ThrottleReasons::HW_SLOWDOWN)
    {
        active.push("HW Slowdown");
    }

    if active.is_empty() {
        (None, false)
    } else {
        (Some(active.join(" + ")), is_thermal)
    }
}

/// Single raw GPU telemetry sample before transient smoothing and throttle debouncing.
#[derive(Debug, Clone, Default)]
pub struct RawGpuSample {
    pub name: Option<String>,
    pub temp_c: Option<f32>,
    pub usage_pct: Option<f32>,
    pub power_w: Option<f32>,
    pub power_limit_w: Option<f32>,
    pub clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub mem_used_mb: Option<u32>,
    pub mem_total_mb: Option<u32>,
    pub pstate: Option<String>,
    pub throttle_reason: Option<String>,
    pub throttle_is_thermal: bool,
}

/// Stabilized GPU metrics and sleep state emitted into [`TelemetrySnapshot`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmoothedGpuOutput {
    pub gpu_name: Option<String>,
    pub gpu_sleeping: bool,
    pub gpu_temp_c: Option<f32>,
    pub gpu_usage_pct: Option<f32>,
    pub gpu_power_w: Option<f32>,
    pub gpu_power_limit_w: Option<f32>,
    pub gpu_clock_mhz: Option<u32>,
    pub gpu_mem_clock_mhz: Option<u32>,
    pub gpu_mem_used_mb: Option<u32>,
    pub gpu_mem_total_mb: Option<u32>,
    pub gpu_pstate: Option<String>,
    pub gpu_throttle_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct HeldField<T> {
    value: Option<T>,
    misses: u8,
}

impl<T> Default for HeldField<T> {
    fn default() -> Self {
        Self {
            value: None,
            misses: 0,
        }
    }
}

impl<T: Clone> HeldField<T> {
    fn clear(&mut self) {
        self.value = None;
        self.misses = 0;
    }

    fn update(&mut self, sample: Option<T>, max_hold_misses: u8) -> Option<T> {
        match sample {
            Some(v) => {
                self.value = Some(v.clone());
                self.misses = 0;
                Some(v)
            }
            None => {
                if self.value.is_some() && self.misses < max_hold_misses {
                    self.misses = self.misses.saturating_add(1);
                    self.value.clone()
                } else {
                    self.value = None;
                    self.misses = 0;
                    None
                }
            }
        }
    }
}

/// Smooths transient NVML sample drops while awake, debounces throttle badges, backs off
/// idle NVML polls so Optimus dGPUs can enter D3cold, and zeroes all metrics cleanly on sleep.
#[derive(Debug, Default)]
pub struct GpuSampleSmoother {
    cached_name: Option<String>,
    temp_c: HeldField<f32>,
    usage_pct: HeldField<f32>,
    power_w: HeldField<f32>,
    power_limit_w: HeldField<f32>,
    clock_mhz: HeldField<u32>,
    mem_clock_mhz: HeldField<u32>,
    mem_used_mb: HeldField<u32>,
    mem_total_mb: HeldField<u32>,
    pstate: HeldField<String>,
    throttle_active: Option<String>,
    pending_throttle: Option<String>,
    pending_throttle_hits: u8,
    throttle_clear_misses: u8,
    empty_core_polls: u8,
    consecutive_idle_polls: u8,
}

impl GpuSampleSmoother {
    const MAX_TRANSIENT_MISSES: u8 = 2;

    fn is_sample_gpu_idle(raw: &RawGpuSample) -> bool {
        raw.usage_pct.is_some_and(|u| u <= 2.0)
            && raw.power_w.is_none_or(|w| w < 20.0)
            && raw.temp_c.is_none_or(|t| t < 65.0)
    }

    fn clear_volatile(&mut self) {
        self.temp_c.clear();
        self.usage_pct.clear();
        self.power_w.clear();
        self.power_limit_w.clear();
        self.clock_mhz.clear();
        self.mem_clock_mhz.clear();
        self.mem_used_mb.clear();
        self.mem_total_mb.clear();
        self.pstate.clear();
        self.throttle_active = None;
        self.pending_throttle = None;
        self.pending_throttle_hits = 0;
        self.throttle_clear_misses = 0;
        self.empty_core_polls = 0;
    }

    pub fn on_sleeping(&mut self) -> SmoothedGpuOutput {
        self.clear_volatile();
        self.consecutive_idle_polls = 0;
        SmoothedGpuOutput {
            gpu_name: self.cached_name.clone(),
            gpu_sleeping: true,
            ..Default::default()
        }
    }

    pub fn on_nvml_paused(&mut self, sleeping: bool) -> SmoothedGpuOutput {
        self.clear_volatile();
        self.consecutive_idle_polls = 0;
        SmoothedGpuOutput {
            gpu_name: self.cached_name.clone(),
            gpu_sleeping: sleeping,
            ..Default::default()
        }
    }

    pub fn current_cached_awake(&self) -> SmoothedGpuOutput {
        SmoothedGpuOutput {
            gpu_name: self.cached_name.clone(),
            gpu_sleeping: false,
            gpu_temp_c: self.temp_c.value,
            gpu_usage_pct: self.usage_pct.value,
            gpu_power_w: self.power_w.value,
            gpu_power_limit_w: self.power_limit_w.value,
            gpu_clock_mhz: self.clock_mhz.value,
            gpu_mem_clock_mhz: self.mem_clock_mhz.value,
            gpu_mem_used_mb: self.mem_used_mb.value,
            gpu_mem_total_mb: self.mem_total_mb.value,
            gpu_pstate: self.pstate.value.clone(),
            gpu_throttle_reason: self.throttle_active.clone(),
        }
    }

    pub fn recommended_nvml_poll_interval(&self, visible: bool) -> Duration {
        if !visible && self.consecutive_idle_polls >= 3 {
            Duration::from_secs(10)
        } else if visible && self.consecutive_idle_polls >= 3 {
            Duration::from_millis(2000)
        } else {
            Duration::ZERO
        }
    }

    pub fn on_awake_sample(&mut self, raw: RawGpuSample) -> SmoothedGpuOutput {
        if raw.name.is_some() {
            self.cached_name = raw.name.clone();
        }

        let all_core_missing = raw.temp_c.is_none()
            && raw.usage_pct.is_none()
            && raw.power_w.is_none()
            && raw.clock_mhz.is_none();
        if all_core_missing {
            self.empty_core_polls = self.empty_core_polls.saturating_add(1);
            if self.empty_core_polls >= 2 {
                self.clear_volatile();
                self.consecutive_idle_polls = 0;
                return SmoothedGpuOutput {
                    gpu_name: self.cached_name.clone(),
                    gpu_sleeping: self.cached_name.is_some(),
                    ..Default::default()
                };
            }
            return self.current_cached_awake();
        }

        self.empty_core_polls = 0;
        if Self::is_sample_gpu_idle(&raw) {
            self.consecutive_idle_polls = self.consecutive_idle_polls.saturating_add(1);
        } else {
            self.consecutive_idle_polls = 0;
        }

        if raw.throttle_is_thermal {
            self.throttle_active = raw.throttle_reason.clone();
            self.pending_throttle = None;
            self.pending_throttle_hits = 0;
            self.throttle_clear_misses = 0;
        } else if let Some(reason) = raw.throttle_reason {
            if self.throttle_active.as_deref() == Some(reason.as_str()) {
                self.throttle_clear_misses = 0;
                self.pending_throttle = None;
                self.pending_throttle_hits = 0;
            } else if self.pending_throttle.as_deref() == Some(reason.as_str()) {
                self.pending_throttle_hits = self.pending_throttle_hits.saturating_add(1);
                if self.pending_throttle_hits >= 2 {
                    self.throttle_active = Some(reason);
                    self.pending_throttle = None;
                    self.pending_throttle_hits = 0;
                    self.throttle_clear_misses = 0;
                }
            } else {
                self.pending_throttle = Some(reason);
                self.pending_throttle_hits = 1;
            }
        } else {
            self.pending_throttle = None;
            self.pending_throttle_hits = 0;
            if self.throttle_active.is_some() {
                self.throttle_clear_misses = self.throttle_clear_misses.saturating_add(1);
                if self.throttle_clear_misses >= 2 {
                    self.throttle_active = None;
                    self.throttle_clear_misses = 0;
                }
            }
        }

        SmoothedGpuOutput {
            gpu_name: self.cached_name.clone(),
            gpu_sleeping: false,
            gpu_temp_c: self.temp_c.update(raw.temp_c, Self::MAX_TRANSIENT_MISSES),
            gpu_usage_pct: self
                .usage_pct
                .update(raw.usage_pct, Self::MAX_TRANSIENT_MISSES),
            gpu_power_w: self.power_w.update(raw.power_w, Self::MAX_TRANSIENT_MISSES),
            gpu_power_limit_w: self
                .power_limit_w
                .update(raw.power_limit_w, Self::MAX_TRANSIENT_MISSES),
            gpu_clock_mhz: self
                .clock_mhz
                .update(raw.clock_mhz, Self::MAX_TRANSIENT_MISSES),
            gpu_mem_clock_mhz: self
                .mem_clock_mhz
                .update(raw.mem_clock_mhz, Self::MAX_TRANSIENT_MISSES),
            gpu_mem_used_mb: self
                .mem_used_mb
                .update(raw.mem_used_mb, Self::MAX_TRANSIENT_MISSES),
            gpu_mem_total_mb: self
                .mem_total_mb
                .update(raw.mem_total_mb, Self::MAX_TRANSIENT_MISSES),
            gpu_pstate: self.pstate.update(raw.pstate, Self::MAX_TRANSIENT_MISSES),
            gpu_throttle_reason: self.throttle_active.clone(),
        }
    }
}

/// Fast (< 0.1 ms) Win32 battery charge percentage lookup without slow WMI queries.
pub fn read_fast_battery_pct() -> Option<u8> {
    WindowsTelemetryBackend::read_win32_power().1
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

type EcWmiPollResult = (
    Option<usize>,
    Vec<u32>,
    Vec<u32>,
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<u32>,
);

impl WindowsTelemetryBackend {
    pub fn new() -> Self {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());
        let mut sys = System::new_with_specifics(refresh);
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let networks = Networks::new_with_refreshed_list();
        let disks =
            Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_io_usage());
        let emi = CpuEmiReader::open();
        let pawnio = PawnIoCpuTempReader::open();

        Self {
            state: Mutex::new(TelemetryState {
                sys,
                networks,
                disks,
                nvml: None,
                nvml_last_attempt: None,
                nvml_attempts: 0,
                last_nvml_poll: None,
                gpu_smoother: GpuSampleSmoother::default(),
                emi,
                pawnio,
                pawnio_last_attempt: None,
                pawnio_attempts: 0,
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
            guard.pawnio_attempts = 0;
            guard.pawnio_last_attempt = None;
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
    ) -> EcWmiPollResult {
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

impl WindowsTelemetryBackend {
    /// Reads a telemetry snapshot with optional throttling for background/tray battery savings.
    ///
    /// - `include_heavy_io == false`: skips disk and network interface refreshes when hidden in tray.
    /// - `allow_nvml == false`: avoids initializing or polling NVML while hidden on battery so an
    ///   idle Optimus dGPU stays asleep in D3cold.
    pub fn read_telemetry_with_options(
        &self,
        include_heavy_io: bool,
        allow_nvml: bool,
    ) -> Result<TelemetrySnapshot> {
        let (
            cpu_usage_pct,
            cpu_temp_c,
            cpu_clock_mhz,
            cpu_power_w,
            ram_used_gb,
            ram_total_gb,
            gpu_name,
            gpu_sleeping,
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

            let (net_rx_mb_s, net_tx_mb_s, disk_read_mb_s, disk_write_mb_s) = if include_heavy_io {
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

                let mut total_rx = 0u64;
                let mut total_tx = 0u64;
                for (iface, data) in &guard.networks {
                    if iface.contains("Loopback") {
                        continue;
                    }
                    total_rx = total_rx.saturating_add(data.received());
                    total_tx = total_tx.saturating_add(data.transmitted());
                }

                let mut total_disk_r = 0u64;
                let mut total_disk_w = 0u64;
                for disk in guard.disks.list() {
                    let usage = disk.usage();
                    total_disk_r = total_disk_r.saturating_add(usage.read_bytes);
                    total_disk_w = total_disk_w.saturating_add(usage.written_bytes);
                }

                (
                    Some((total_rx as f32 / (1024.0 * 1024.0)) / dt_secs),
                    Some((total_tx as f32 / (1024.0 * 1024.0)) / dt_secs),
                    Some((total_disk_r as f32 / (1024.0 * 1024.0)) / dt_secs),
                    Some((total_disk_w as f32 / (1024.0 * 1024.0)) / dt_secs),
                )
            } else {
                (None, None, None, None)
            };
            guard.last_fast_refresh = Instant::now();

            let cpu_usage_pct = Some(guard.sys.global_cpu_usage());
            let cpu_clock_mhz = guard.sys.cpus().first().map(|c| c.frequency() as u32);
            let ram_total_gb = Some(guard.sys.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0));
            let ram_used_gb = Some(guard.sys.used_memory() as f32 / (1024.0 * 1024.0 * 1024.0));

            let cpu_power_w = guard.emi.as_mut().and_then(|e| e.sample_cpu_power_w());

            if guard.pawnio.is_none()
                && crate::pawnio::has_intel_msr_module()
                && should_retry_init(
                    guard.pawnio_attempts,
                    3,
                    guard.pawnio_last_attempt,
                    Duration::from_secs(30),
                )
            {
                guard.pawnio_attempts = guard.pawnio_attempts.saturating_add(1);
                guard.pawnio_last_attempt = Some(Instant::now());
                guard.pawnio = PawnIoCpuTempReader::open();
            }
            let cpu_temp_c = guard.pawnio.as_mut().and_then(|p| p.read_package_temp_c());

            let pnp_state = query_nvidia_pnp_power_state();
            let gpu_out = match pnp_state {
                GpuPowerState::SleepingD3 => guard.gpu_smoother.on_sleeping(),
                _ if !allow_nvml => guard.gpu_smoother.on_nvml_paused(false),
                _ => {
                    let min_interval = guard
                        .gpu_smoother
                        .recommended_nvml_poll_interval(include_heavy_io);
                    if guard.nvml.is_some()
                        && !min_interval.is_zero()
                        && guard
                            .last_nvml_poll
                            .is_some_and(|t| t.elapsed() < min_interval)
                    {
                        guard.gpu_smoother.current_cached_awake()
                    } else {
                        if guard.nvml.is_none()
                            && should_retry_init(
                                guard.nvml_attempts,
                                5,
                                guard.nvml_last_attempt,
                                Duration::from_secs(30),
                            )
                        {
                            guard.nvml_attempts = guard.nvml_attempts.saturating_add(1);
                            guard.nvml_last_attempt = Some(Instant::now());
                            let nvml_path = crate::paths::system_tool("nvml.dll");
                            if !nvml_path.is_file() {
                                tracing::debug!(
                                    path = %nvml_path.display(),
                                    "System32 nvml.dll not present; disabling NVML telemetry"
                                );
                                guard.nvml_attempts = 5;
                            } else {
                                match Nvml::builder().lib_path(nvml_path.as_os_str()).init() {
                                    Ok(nvml) => {
                                        tracing::info!(
                                            path = %nvml_path.display(),
                                            "Initialized NVIDIA NVML telemetry"
                                        );
                                        guard.nvml = Some(nvml);
                                    }
                                    Err(err) => {
                                        tracing::debug!(
                                            attempt = guard.nvml_attempts,
                                            "NVML initialization skipped/unavailable: {err}"
                                        );
                                    }
                                }
                            }
                        }

                        if guard.nvml.is_none() {
                            guard.gpu_smoother.on_nvml_paused(false)
                        } else {
                            let mut raw = RawGpuSample::default();
                            if let Some(nvml) = &guard.nvml {
                                if let Ok(device) = nvml.device_by_index(0) {
                                    raw.name = device.name().ok();
                                    raw.temp_c = device
                                        .temperature(TemperatureSensor::Gpu)
                                        .ok()
                                        .and_then(|t| sanitize_nvml_temp_c(t as f32));
                                    raw.usage_pct = match device.utilization_rates() {
                                        Ok(u) => sanitize_nvml_usage_pct(u.gpu),
                                        Err(err) => {
                                            tracing::debug!(
                                                "NVML utilization_rates unavailable: {err}"
                                            );
                                            None
                                        }
                                    };
                                    raw.power_limit_w = device
                                        .enforced_power_limit()
                                        .or_else(|_| device.power_management_limit())
                                        .ok()
                                        .map(|mw| mw as f32 / 1000.0)
                                        .filter(|w| w.is_finite() && (10.0..=600.0).contains(w));
                                    raw.power_w = device.power_usage().ok().and_then(|mw| {
                                        let raw_w = mw as f32 / 1000.0;
                                        let valid = sanitize_nvml_power_w(raw_w, raw.power_limit_w);
                                        if valid.is_none() {
                                            tracing::debug!(
                                                raw_w,
                                                power_limit_w = ?raw.power_limit_w,
                                                "Discarded implausible NVML GPU power sample"
                                            );
                                        }
                                        valid
                                    });
                                    raw.clock_mhz = device.clock_info(Clock::Graphics).ok();
                                    raw.mem_clock_mhz = device.clock_info(Clock::Memory).ok();
                                    if let Ok(mem) = device.memory_info() {
                                        raw.mem_used_mb = Some((mem.used / (1024 * 1024)) as u32);
                                        raw.mem_total_mb = Some((mem.total / (1024 * 1024)) as u32);
                                    }
                                    raw.pstate = device.performance_state().ok().map(format_pstate);
                                    if let Ok(reasons) = device.current_throttle_reasons() {
                                        let (reason, is_thermal) = filter_throttle_reasons(
                                            reasons,
                                            raw.usage_pct,
                                            raw.power_w,
                                            raw.temp_c,
                                        );
                                        raw.throttle_reason = reason;
                                        raw.throttle_is_thermal = is_thermal;
                                    }
                                }
                            }
                            guard.last_nvml_poll = Some(Instant::now());
                            guard.gpu_smoother.on_awake_sample(raw)
                        }
                    }
                }
            };

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
                gpu_out.gpu_name,
                gpu_out.gpu_sleeping,
                gpu_out.gpu_temp_c,
                gpu_out.gpu_usage_pct,
                gpu_out.gpu_power_w,
                gpu_out.gpu_power_limit_w,
                gpu_out.gpu_clock_mhz,
                gpu_out.gpu_mem_clock_mhz,
                gpu_out.gpu_mem_used_mb,
                gpu_out.gpu_mem_total_mb,
                gpu_out.gpu_pstate,
                gpu_out.gpu_throttle_reason,
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
            gpu_sleeping,
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

impl TelemetryBackend for WindowsTelemetryBackend {
    fn read_telemetry(&self) -> Result<TelemetrySnapshot> {
        self.read_telemetry_with_options(true, true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_init_respects_cooldown_and_max_attempts() {
        let cd = Duration::from_secs(30);
        // First attempt allowed
        assert!(should_retry_init(0, 5, None, cd));
        // Inside cooldown -> blocked
        let recent = Some(Instant::now());
        assert!(!should_retry_init(1, 5, recent, cd));
        // Past cooldown -> allowed
        let old = Some(Instant::now() - Duration::from_secs(31));
        assert!(should_retry_init(1, 5, old, cd));
        // Reached max attempts -> blocked permanently
        assert!(!should_retry_init(5, 5, old, cd));
    }

    #[test]
    fn sanitize_nvml_filters_d3cold_power_spikes_and_invalid_metrics() {
        assert_eq!(sanitize_nvml_power_w(590.0, Some(80.0)), None);
        assert_eq!(sanitize_nvml_power_w(590.0, None), None);
        assert_eq!(sanitize_nvml_power_w(-5.0, Some(140.0)), None);
        assert_eq!(sanitize_nvml_power_w(16.2, Some(80.0)), Some(16.2));
        assert_eq!(sanitize_nvml_power_w(140.0, Some(140.0)), Some(140.0));
        assert_eq!(sanitize_nvml_temp_c(51.0), Some(51.0));
        assert_eq!(sanitize_nvml_temp_c(0.0), None);
        assert_eq!(sanitize_nvml_usage_pct(42), Some(42.0));
        assert_eq!(sanitize_nvml_usage_pct(150), None);
    }

    #[test]
    fn parse_cm_power_data_state_decodes_d0_d3_and_invalid_buffers() {
        let make_buf = |size: u32, state: u32| -> Vec<u8> {
            let mut b = vec![0u8; 56];
            b[0..4].copy_from_slice(&size.to_le_bytes());
            b[4..8].copy_from_slice(&state.to_le_bytes());
            b
        };
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 1)),
            GpuPowerState::AwakeD0
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 4)),
            GpuPowerState::SleepingD3
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 2)),
            GpuPowerState::SleepingD3
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 3)),
            GpuPowerState::SleepingD3
        );
        assert_eq!(
            parse_cm_power_data_state(&[56, 0, 0, 0, 1, 0, 0]),
            GpuPowerState::Unknown
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(0, 1)),
            GpuPowerState::Unknown
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 0)),
            GpuPowerState::Unknown
        );
        assert_eq!(
            parse_cm_power_data_state(&make_buf(56, 5)),
            GpuPowerState::Unknown
        );
    }

    #[test]
    fn filter_throttle_reasons_suppresses_idle_power_noise_but_keeps_thermal_and_load() {
        // Idle P-state transition SW_POWER_CAP at 0% / 15W -> suppressed
        assert_eq!(
            filter_throttle_reasons(
                ThrottleReasons::SW_POWER_CAP,
                Some(0.0),
                Some(15.0),
                Some(46.0),
            ),
            (None, false)
        );
        // Real SW_POWER_CAP under load -> surfaced as Power
        assert_eq!(
            filter_throttle_reasons(
                ThrottleReasons::SW_POWER_CAP,
                Some(55.0),
                Some(70.0),
                Some(62.0),
            ),
            (Some("Power".to_string()), false)
        );
        // Thermal slowdown NEVER suppressed even at 0% load
        assert_eq!(
            filter_throttle_reasons(
                ThrottleReasons::HW_THERMAL_SLOWDOWN,
                Some(0.0),
                Some(10.0),
                Some(88.0),
            ),
            (Some("Thermal".to_string()), true)
        );
        // Idle HW_SLOWDOWN at cool 0%/45C -> suppressed; hot >=75C -> surfaced
        assert_eq!(
            filter_throttle_reasons(
                ThrottleReasons::HW_SLOWDOWN,
                Some(0.0),
                Some(12.0),
                Some(45.0),
            ),
            (None, false)
        );
        assert_eq!(
            filter_throttle_reasons(
                ThrottleReasons::HW_SLOWDOWN,
                Some(0.0),
                Some(12.0),
                Some(80.0),
            ),
            (Some("HW Slowdown".to_string()), false)
        );
    }

    #[test]
    fn gpu_smoother_holds_transient_misses_clears_immediately_on_sleep_and_debounces_throttle() {
        let mut smoother = GpuSampleSmoother::default();
        let base_sample = RawGpuSample {
            name: Some("RTX 4070 Laptop GPU".to_string()),
            temp_c: Some(49.0),
            usage_pct: Some(25.0),
            power_w: Some(42.5),
            power_limit_w: Some(90.0),
            clock_mhz: Some(1980),
            mem_clock_mhz: Some(8001),
            mem_used_mb: Some(512),
            mem_total_mb: Some(8188),
            pstate: Some("P0".to_string()),
            throttle_reason: None,
            throttle_is_thermal: false,
        };
        let out0 = smoother.on_awake_sample(base_sample.clone());
        assert!(!out0.gpu_sleeping);
        assert_eq!(out0.gpu_power_w, Some(42.5));
        assert_eq!(out0.gpu_throttle_reason, None);

        // 1-tick Power throttle flicker is ignored by 2-hit debounce
        let mut one_tick_power = base_sample.clone();
        one_tick_power.throttle_reason = Some("Power".to_string());
        let out1 = smoother.on_awake_sample(one_tick_power);
        assert_eq!(out1.gpu_throttle_reason, None);

        // 2 consecutive Power throttle samples engage Power throttle
        let mut hit1 = base_sample.clone();
        hit1.throttle_reason = Some("Power".to_string());
        let _ = smoother.on_awake_sample(hit1.clone());
        let out2 = smoother.on_awake_sample(hit1);
        assert_eq!(out2.gpu_throttle_reason.as_deref(), Some("Power"));

        // 1-2 transient missing power readings hold previous 42.5W; 3rd miss clears to None
        let mut miss_power = base_sample.clone();
        miss_power.power_w = None;
        assert_eq!(
            smoother.on_awake_sample(miss_power.clone()).gpu_power_w,
            Some(42.5)
        );
        assert_eq!(
            smoother.on_awake_sample(miss_power.clone()).gpu_power_w,
            Some(42.5)
        );
        assert_eq!(smoother.on_awake_sample(miss_power).gpu_power_w, None);

        // Thermal throttle engages immediately on the first hit
        let mut thermal_hit = base_sample;
        thermal_hit.throttle_reason = Some("Thermal".to_string());
        thermal_hit.throttle_is_thermal = true;
        let out_th = smoother.on_awake_sample(thermal_hit);
        assert_eq!(out_th.gpu_throttle_reason.as_deref(), Some("Thermal"));

        // D3 sleep clears all metrics immediately without waiting for misses
        let out_sleep = smoother.on_sleeping();
        assert!(out_sleep.gpu_sleeping);
        assert_eq!(out_sleep.gpu_name.as_deref(), Some("RTX 4070 Laptop GPU"));
        assert_eq!(out_sleep.gpu_temp_c, None);
        assert_eq!(out_sleep.gpu_power_w, None);
        assert_eq!(out_sleep.gpu_pstate, None);
        assert_eq!(out_sleep.gpu_throttle_reason, None);

        // Re-wake then 2 consecutive empty awake polls clear all metrics together
        let wake = RawGpuSample {
            temp_c: Some(46.0),
            usage_pct: Some(5.0),
            power_w: Some(15.0),
            clock_mhz: Some(210),
            ..Default::default()
        };
        assert!(!smoother.on_awake_sample(wake).gpu_sleeping);
        let empty = RawGpuSample::default();
        let hold1 = smoother.on_awake_sample(empty.clone());
        assert!(!hold1.gpu_sleeping);
        assert_eq!(hold1.gpu_temp_c, Some(46.0));
        let sleep2 = smoother.on_awake_sample(empty);
        assert!(sleep2.gpu_sleeping);
        assert_eq!(sleep2.gpu_temp_c, None);
    }

    #[test]
    fn gpu_smoother_recommends_idle_backoff_when_idle_and_fast_when_active() {
        let mut smoother = GpuSampleSmoother::default();
        let idle = RawGpuSample {
            temp_c: Some(44.0),
            usage_pct: Some(0.0),
            power_w: Some(2.0),
            clock_mhz: Some(210),
            ..Default::default()
        };
        assert_eq!(
            smoother.recommended_nvml_poll_interval(true),
            Duration::ZERO
        );
        for _ in 0..2 {
            smoother.on_awake_sample(idle.clone());
            assert_eq!(
                smoother.recommended_nvml_poll_interval(true),
                Duration::ZERO
            );
        }
        smoother.on_awake_sample(idle);
        assert_eq!(
            smoother.recommended_nvml_poll_interval(true),
            Duration::from_millis(2000)
        );
        assert_eq!(
            smoother.recommended_nvml_poll_interval(false),
            Duration::from_secs(10)
        );

        // Active sample resets idle backoff immediately
        let active = RawGpuSample {
            temp_c: Some(58.0),
            usage_pct: Some(35.0),
            power_w: Some(55.0),
            clock_mhz: Some(1980),
            ..Default::default()
        };
        smoother.on_awake_sample(active);
        assert_eq!(
            smoother.recommended_nvml_poll_interval(true),
            Duration::ZERO
        );
        assert_eq!(
            smoother.recommended_nvml_poll_interval(false),
            Duration::ZERO
        );
    }
}
