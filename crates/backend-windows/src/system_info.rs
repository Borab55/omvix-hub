use crate::wmi_com::{WmiConnection, get_string, get_u32, get_u32_array};
use omvix_core::{ConflictServiceInfo, Result, SecurityStatus, SystemIdentity};
use std::path::Path;
use windows::Win32::UI::Shell::IsUserAnAdmin;

const MONITORED_SERVICES: &[(&str, &str)] = &[
    ("HPOmenCap", "HP HSA / OGH"),
    ("HPAppHelperCap", "HP HSA"),
    ("HPDiagsCap", "HP HSA"),
    ("HPNetworkCap", "HP HSA"),
    ("HPSysInfoCap", "HP HSA"),
    ("HpTouchpointAnalyticsService", "HP Telemetry"),
    ("XTU3SERVICE", "Intel XTU"),
    ("hp-charge-limit-rs", "Battery Charge Limit"),
];

pub fn collect_system_identity() -> Result<SystemIdentity> {
    let cim = WmiConnection::connect(r"root\CIMV2")?;

    let (manufacturer, model, system_family) = cim
        .query("SELECT Manufacturer, Model, SystemFamily FROM Win32_ComputerSystem")?
        .into_iter()
        .next()
        .map(|o| {
            (
                get_string(&o, "Manufacturer").unwrap_or_default(),
                get_string(&o, "Model").unwrap_or_default(),
                get_string(&o, "SystemFamily").unwrap_or_default(),
            )
        })
        .unwrap_or_default();

    let board_id = cim
        .query("SELECT Product FROM Win32_BaseBoard")?
        .into_iter()
        .next()
        .and_then(|o| get_string(&o, "Product"))
        .unwrap_or_default();

    let bios_version = cim
        .query("SELECT SMBIOSBIOSVersion FROM Win32_BIOS")?
        .into_iter()
        .next()
        .and_then(|o| get_string(&o, "SMBIOSBIOSVersion"))
        .unwrap_or_default();

    let cpu_name = cim
        .query("SELECT Name FROM Win32_Processor")?
        .into_iter()
        .next()
        .and_then(|o| get_string(&o, "Name"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let gpu_names = cim
        .query("SELECT Name FROM Win32_VideoController")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|o| get_string(&o, "Name"))
        .collect();

    let os_name = cim
        .query("SELECT Caption, Version FROM Win32_OperatingSystem")
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|o| {
            let cap = get_string(&o, "Caption").unwrap_or_else(|| "Windows".to_string());
            let ver = get_string(&o, "Version").unwrap_or_default();
            if ver.is_empty() {
                cap
            } else {
                format!("{cap} ({ver})")
            }
        })
        .unwrap_or_else(|| "Windows".to_string());

    Ok(SystemIdentity {
        manufacturer,
        model,
        system_family,
        board_id,
        bios_version,
        cpu_name,
        gpu_names,
        os_name,
    })
}

/// Fast synchronous check (< 0.01 ms) of current process elevation.
pub fn is_process_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

pub fn collect_security_status() -> SecurityStatus {
    let is_admin = is_process_elevated();
    let mut vbs_status = None;
    let mut hvci_running = false;

    if let Ok(dg) = WmiConnection::connect(r"root\Microsoft\Windows\DeviceGuard") {
        if let Ok(rows) = dg.query(
            "SELECT VirtualizationBasedSecurityStatus, SecurityServicesRunning FROM Win32_DeviceGuard",
        ) {
            if let Some(row) = rows.into_iter().next() {
                vbs_status = get_u32(&row, "VirtualizationBasedSecurityStatus");
                if let Some(services) = get_u32_array(&row, "SecurityServicesRunning") {
                    // 2 = Hypervisor-enforced Code Integrity (Memory Integrity / HVCI)
                    hvci_running = services.contains(&2);
                }
            }
        }
    }

    let pawn_io_installed = Path::new(r"C:\Program Files\PawnIO\PawnIOUtil.exe").exists()
        || check_pawnio_driver();

    let undervolt_note = if hvci_running || vbs_status == Some(2) {
        "VBS / Memory Integrity (HVCI) is active: direct ring-0 MSR undervolting (0x150) is blocked by the Windows hypervisor. WMI/Win32 thermal, fan, boost, and battery limit controls remain available.".to_string()
    } else if pawn_io_installed {
        "VBS/HVCI is inactive and PawnIO is installed: MSR undervolt/TCC/PL controls can be enabled.".to_string()
    } else {
        "VBS/HVCI appears inactive; optional PawnIO driver can be used for direct MSR undervolt/TCC/PL tuning.".to_string()
    };

    SecurityStatus {
        is_admin,
        vbs_status,
        hvci_running,
        pawn_io_installed,
        undervolt_note,
    }
}

fn check_pawnio_driver() -> bool {
    let Ok(cim) = WmiConnection::connect(r"root\CIMV2") else {
        return false;
    };
    cim.query("SELECT Name FROM Win32_SystemDriver WHERE Name LIKE '%PawnIO%'")
        .map(|rows| !rows.is_empty())
        .unwrap_or(false)
}

pub fn check_hp_wmi_classes() -> (bool, bool) {
    match WmiConnection::connect(r"root\WMI") {
        Ok(wmi) => (wmi.class_exists("hpqBIntM"), wmi.class_exists("hpqBEvnt")),
        Err(err) => {
            tracing::warn!("Could not connect to root\\WMI to inspect HP classes: {err}");
            (false, false)
        }
    }
}

pub fn scan_conflict_services() -> Vec<ConflictServiceInfo> {
    let Ok(cim) = WmiConnection::connect(r"root\CIMV2") else {
        return Vec::new();
    };
    let Ok(rows) = cim.query("SELECT Name, DisplayName, State FROM Win32_Service") else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for row in rows {
        let Some(name) = get_string(&row, "Name") else {
            continue;
        };
        if let Some((_, category)) = MONITORED_SERVICES
            .iter()
            .find(|(target, _)| target.eq_ignore_ascii_case(&name))
        {
            let display_name = get_string(&row, "DisplayName").unwrap_or_else(|| name.clone());
            let status = get_string(&row, "State").unwrap_or_else(|| "Unknown".to_string());
            found.push(ConflictServiceInfo {
                service_name: name,
                display_name,
                status,
                category: (*category).to_string(),
            });
        }
    }
    found.sort_by(|a, b| a.service_name.cmp(&b.service_name));
    found
}
