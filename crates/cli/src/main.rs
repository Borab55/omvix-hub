use clap::{Parser, Subcommand, ValueEnum};
use omvix_core::{
    AppConfig, BatteryChargeMode, BatteryLimitBackend, BatteryStatus, DiagnosticReport,
    DiagnosticsBackend, DisplayBackend, FanMode, LogLevel, PerformanceMode, TelemetryBackend,
    TelemetrySnapshot, ThermalFanBackend, init_logging, list_log_files, total_log_bytes,
};
use omvix_windows::{
    WindowsBatteryBackend, WindowsCpuTuningBackend, WindowsDiagnosticsBackend,
    WindowsDisplayBackend, WindowsTelemetryBackend, WindowsThermalFanBackend, system_info,
};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "omvix-cli",
    version,
    about = "Omvix Hub hardware probe, diagnostics, and CLI utility"
)]
struct Cli {
    /// Print debug-level log events to stderr while running.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a strictly read-only HP WMI & system hardware diagnostic probe.
    Probe,
    /// Export full hardware diagnostic report as JSON.
    ExportDiagnostics {
        /// Output JSON file path.
        #[arg(long, default_value = "diagnostics.json")]
        out: PathBuf,
    },
    /// Show active rotating log directory and disk usage.
    LogsPath,
    /// Read live system telemetry snapshot (CPU, NVIDIA GPU, RAM, fans, battery).
    Telemetry,
    /// Thermal / performance mode controls (requires Administrator).
    Thermal {
        #[command(subcommand)]
        action: ThermalAction,
    },
    /// Cooling fan controls (requires Administrator).
    Fan {
        #[command(subcommand)]
        action: FanAction,
    },
    /// Battery health and WMI 0x2B charge limiting controls.
    Battery {
        #[command(subcommand)]
        action: BatteryAction,
    },
    /// Display refresh rate diagnostics and switching.
    Display {
        #[command(subcommand)]
        action: DisplayAction,
    },
    /// CPU Boost and VBS/HVCI undervolt tuning status.
    Tuning {
        #[command(subcommand)]
        action: TuningAction,
    },
}

#[derive(Subcommand)]
enum DisplayAction {
    /// List current resolution and supported refresh rates (Hz).
    List,
}

#[derive(Subcommand)]
enum TuningAction {
    /// Show CPU Turbo Boost mode and VBS/HVCI/PawnIO undervolt lock status.
    Status,
}

#[derive(Subcommand)]
enum BatteryAction {
    /// Show battery level, capacity, wear, charge rate, and active charge mode.
    Status,
    /// Set battery charge-limiting mode (requires Administrator).
    SetMode {
        #[arg(value_enum)]
        mode: CliBatteryMode,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliBatteryMode {
    Auto,
    Inhibit,
    Hold,
}

impl From<CliBatteryMode> for BatteryChargeMode {
    fn from(value: CliBatteryMode) -> Self {
        match value {
            CliBatteryMode::Auto => Self::Auto,
            CliBatteryMode::Inhibit => Self::Inhibit,
            CliBatteryMode::Hold => Self::Hold,
        }
    }
}

#[derive(Subcommand)]
enum ThermalAction {
    /// Set active thermal/performance mode.
    Set {
        #[arg(value_enum)]
        mode: CliPerfMode,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliPerfMode {
    Eco,
    Quiet,
    Balanced,
    Performance,
}

impl From<CliPerfMode> for PerformanceMode {
    fn from(value: CliPerfMode) -> Self {
        match value {
            CliPerfMode::Eco => Self::Eco,
            CliPerfMode::Quiet => Self::Quiet,
            CliPerfMode::Balanced => Self::Balanced,
            CliPerfMode::Performance => Self::Performance,
        }
    }
}

#[derive(Subcommand)]
enum FanAction {
    /// Set cooling fan control mode.
    Set {
        #[arg(value_enum)]
        mode: CliFanMode,
        /// Target fan speed percentage (0..100) for manual mode.
        #[arg(long)]
        speed: Option<u8>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliFanMode {
    Auto,
    Max,
    Manual,
}

impl From<CliFanMode> for FanMode {
    fn from(value: CliFanMode) -> Self {
        match value {
            CliFanMode::Auto => Self::Auto,
            CliFanMode::Max => Self::Max,
            CliFanMode::Manual => Self::Manual,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg_path = AppConfig::default_config_path();
    let mut cfg = AppConfig::load_from_path(&cfg_path).unwrap_or_default();
    if cli.verbose {
        cfg.logging.level = LogLevel::Debug;
    }

    let log_handle = match init_logging(&cfg.logging, cli.verbose) {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("Warning: could not initialize rotating file logger: {err}");
            None
        }
    };

    match cli.command {
        Commands::Probe => match run_diagnostics() {
            Ok(report) => {
                print_report(&report);
                if let Some(h) = log_handle {
                    h.flush();
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                tracing::error!("Probe failed: {err}");
                eprintln!("Error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::ExportDiagnostics { out } => match run_diagnostics() {
            Ok(report) => match serde_json::to_string_pretty(&report) {
                Ok(json) => {
                    if let Err(err) = fs::write(&out, json) {
                        eprintln!("Failed to write {}: {err}", out.display());
                        ExitCode::FAILURE
                    } else {
                        tracing::info!(path = %out.display(), "Exported diagnostic report");
                        println!("Exported diagnostic report to {}", out.display());
                        ExitCode::SUCCESS
                    }
                }
                Err(err) => {
                    eprintln!("Failed to serialize report: {err}");
                    ExitCode::FAILURE
                }
            },
            Err(err) => {
                tracing::error!("Export diagnostics failed: {err}");
                eprintln!("Error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::LogsPath => {
            let dir = cfg.logging.resolved_dir();
            let files = list_log_files(&dir);
            let total_bytes = total_log_bytes(&dir);
            println!("Log directory : {}", dir.display());
            println!("Active level  : {:?}", cfg.logging.level);
            println!(
                "Disk usage    : {} bytes ({:.2} KB) / {} MB cap ({} files)",
                total_bytes,
                total_bytes as f64 / 1024.0,
                cfg.logging.max_total_mb,
                files.len()
            );
            for file in files {
                let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                println!("  - {} ({} bytes)", file.display(), size);
            }
            ExitCode::SUCCESS
        }
        Commands::Telemetry => match WindowsTelemetryBackend::new().read_telemetry() {
            Ok(snap) => {
                print_telemetry(&snap);
                ExitCode::SUCCESS
            }
            Err(err) => {
                tracing::error!("Telemetry read failed: {err}");
                eprintln!("Error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Thermal { action } => match action {
            ThermalAction::Set { mode } => {
                let target: PerformanceMode = mode.into();
                match WindowsThermalFanBackend::new().set_performance_mode(target) {
                    Ok(()) => {
                        println!("Applied performance mode: {target:?}");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        tracing::error!("Failed to set performance mode: {err}");
                        eprintln!("Error: {err}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
        Commands::Fan { action } => match action {
            FanAction::Set { mode, speed } => {
                let target: FanMode = mode.into();
                match WindowsThermalFanBackend::new().set_fan_mode(target, speed) {
                    Ok(()) => {
                        println!("Applied fan mode: {target:?} (speed={speed:?})");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        tracing::error!("Failed to set fan mode: {err}");
                        eprintln!("Error: {err}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
        Commands::Battery { action } => match action {
            BatteryAction::Status => match WindowsBatteryBackend::new().get_battery_status() {
                Ok(st) => {
                    print_battery_status(&st);
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    tracing::error!("Failed to read battery status: {err}");
                    eprintln!("Error: {err}");
                    ExitCode::FAILURE
                }
            },
            BatteryAction::SetMode { mode } => {
                let target: BatteryChargeMode = mode.into();
                match WindowsBatteryBackend::new().set_charge_mode(target) {
                    Ok(()) => {
                        println!("Applied battery charge mode: {target:?}");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        tracing::error!("Failed to set battery charge mode: {err}");
                        eprintln!("Error: {err}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
        Commands::Display { action } => match action {
            DisplayAction::List => match WindowsDisplayBackend::new().get_display_mode() {
                Ok(info) => {
                    println!("============================================================");
                    println!(" Omvix Hub Display Modes");
                    println!("============================================================");
                    println!("Resolution   : {}x{}", info.width, info.height);
                    println!("Current Hz   : {} Hz", info.current_hz);
                    let hz_list = info
                        .supported_hz
                        .iter()
                        .map(|h| format!("{h} Hz"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Supported Hz : {hz_list}");
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    tracing::error!("Failed to enumerate display modes: {err}");
                    eprintln!("Error: {err}");
                    ExitCode::FAILURE
                }
            },
        },
        Commands::Tuning { action } => match action {
            TuningAction::Status => {
                let sec = system_info::collect_security_status();
                let boost = WindowsCpuTuningBackend::new().read_cpu_boost_mode();
                println!("============================================================");
                println!(" Omvix Hub CPU Tuning Status");
                println!("============================================================");
                println!(
                    "CPU Boost    : {}",
                    boost
                        .map(|b| format!("{b:?}"))
                        .unwrap_or_else(|e| format!("Unknown ({e})"))
                );
                println!("Admin        : {}", sec.is_admin);
                println!(
                    "VBS Status   : {} (HVCI: {})",
                    sec.vbs_status
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    sec.hvci_running
                );
                println!("PawnIO Ready : {}", sec.pawn_io_installed);
                println!("Tuning Note  : {}", sec.undervolt_note);
                ExitCode::SUCCESS
            }
        },
    }
}

fn print_battery_status(b: &BatteryStatus) {
    println!("============================================================");
    println!(" Omvix Hub Battery Care Status");
    println!("============================================================");
    println!("Present        : {}", b.present);
    println!("AC Connected   : {}", b.ac_connected);
    println!("Charging       : {}", b.charging);
    println!("Charge Level   : {}%", b.charge_percent);
    println!(
        "Power Rate     : {}",
        b.charge_rate_w
            .map(|w| format!("{w:+.2} W"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Voltage        : {}",
        b.voltage_v
            .map(|v| format!("{v:.2} V"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Remaining Cap  : {}",
        b.remaining_capacity_mwh
            .map(|m| format!("{m} mWh"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Time Remaining : {}",
        b.time_remaining_min
            .map(|m| format!("{}h {}m", m / 60, m % 60))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Design Cap     : {}",
        b.design_capacity_mwh
            .map(|m| format!("{m} mWh"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Full Charge    : {}",
        b.full_charge_capacity_mwh
            .map(|m| format!("{m} mWh"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Battery Wear   : {}",
        b.wear_percent
            .map(|w| format!("{w:.1}%"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Firmware Mode  : {:?}{}",
        b.current_mode,
        b.raw_mode_hex
            .as_deref()
            .map(|h| format!(" [{h}]"))
            .unwrap_or_default()
    );
}

fn print_telemetry(t: &TelemetrySnapshot) {
    println!("============================================================");
    println!(" Omvix Hub Live Telemetry");
    println!("============================================================");
    println!(
        "CPU Usage   : {}",
        t.cpu_usage_pct
            .map(|v| format!("{v:.1}%"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "CPU Clock   : {}",
        t.cpu_clock_mhz
            .map(|v| format!("{v} MHz"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "CPU Power   : {}",
        t.cpu_power_w
            .map(|v| format!("{v:.1} W (EMI)"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "CPU Temp    : {}",
        t.cpu_temp_c
            .map(|v| format!("{v:.1} °C"))
            .unwrap_or_else(|| "N/A (requires Admin/WMI)".to_string())
    );
    println!(
        "GPU         : {}",
        t.gpu_name.as_deref().unwrap_or("N/A")
    );
    println!(
        "GPU Temp/Ld : {} / {}",
        t.gpu_temp_c
            .map(|v| format!("{v:.1} °C"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_usage_pct
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "GPU Pwr/Lim : {} / {}",
        t.gpu_power_w
            .map(|v| format!("{v:.1} W"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_power_limit_w
            .map(|v| format!("{v:.0} W"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "GPU Clocks  : {} core / {} mem ({})",
        t.gpu_clock_mhz
            .map(|v| format!("{v} MHz"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_mem_clock_mhz
            .map(|v| format!("{v} MHz"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_pstate.as_deref().unwrap_or("?")
    );
    println!(
        "GPU VRAM    : {} / {} ({})",
        t.gpu_mem_used_mb
            .map(|v| format!("{v} MB"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_mem_total_mb
            .map(|v| format!("{v} MB"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.gpu_throttle_reason.as_deref().unwrap_or("No Throttle")
    );
    println!(
        "RAM         : {} / {}",
        t.ram_used_gb
            .map(|v| format!("{v:.2} GB"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.ram_total_gb
            .map(|v| format!("{v:.2} GB"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Disk I/O    : R {} | W {}",
        t.disk_read_mb_s
            .map(|v| format!("{v:.2} MB/s"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.disk_write_mb_s
            .map(|v| format!("{v:.2} MB/s"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Network I/O : Down {} | Up {}",
        t.net_rx_mb_s
            .map(|v| format!("{v:.2} MB/s"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.net_tx_mb_s
            .map(|v| format!("{v:.2} MB/s"))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "Fan RPMs    : {}",
        if t.fan_rpms.is_empty() {
            "N/A (requires Admin/WMI)".to_string()
        } else {
            t.fan_rpms
                .iter()
                .map(|r| format!("{r} RPM"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    println!(
        "Power / Bat : {} | {} | Rate: {} | Volt: {}",
        match t.ac_connected {
            Some(true) => "AC Plugged In",
            Some(false) => "On Battery",
            None => "Unknown",
        },
        t.battery_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.battery_rate_w
            .map(|w| format!("{w:+.2} W"))
            .unwrap_or_else(|| "N/A".to_string()),
        t.battery_voltage_v
            .map(|v| format!("{v:.2} V"))
            .unwrap_or_else(|| "N/A".to_string())
    );
}

fn run_diagnostics() -> omvix_core::Result<DiagnosticReport> {
    tracing::info!("Starting read-only Omvix hardware diagnostics");
    let backend = WindowsDiagnosticsBackend::new();
    backend.collect_diagnostics()
}

fn print_report(r: &DiagnosticReport) {
    println!("============================================================");
    println!(" Omvix Hub Hardware Probe (v{})", r.app_version);
    println!("============================================================");
    println!("Manufacturer : {}", r.identity.manufacturer);
    println!("Model        : {}", r.identity.model);
    println!("Family       : {}", r.identity.system_family);
    println!("Board ID     : {}", r.identity.board_id);
    println!("BIOS Version : {}", r.identity.bios_version);
    println!("CPU          : {}", r.identity.cpu_name);
    println!("GPUs         : {}", r.identity.gpu_names.join(" | "));
    println!("OS           : {}", r.identity.os_name);

    println!("\n--- Board Database Match ---");
    if let Some(b) = &r.matched_board {
        println!(
            "Matched      : {} ({:?}, MUX={}, Kbd={:?}, ChargeLimit={})",
            b.marketing_name,
            b.thermal_scheme,
            b.expected_has_mux,
            b.expected_keyboard,
            b.expected_charge_limit
        );
        println!("Notes        : {}", b.notes);
    } else {
        println!("Matched      : None (unknown board ID `{}`)", r.identity.board_id);
    }

    println!("\n--- Security & Privileges ---");
    println!("Elevated Admin : {}", r.security.is_admin);
    println!(
        "VBS Status     : {} (HVCI Running: {})",
        r.security
            .vbs_status
            .map(|v| v.to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
        r.security.hvci_running
    );
    println!("PawnIO Present : {}", r.security.pawn_io_installed);
    println!("Undervolt Note : {}", r.security.undervolt_note);

    println!("\n--- HP WMI Interface ---");
    println!("hpqBIntM Class : {}", r.wmi_hpqbintm_available);
    println!("hpqBEvnt Class : {}", r.wmi_hpqbevnt_available);
    if let Some(err) = &r.wmi_error {
        println!("WMI Status     : {err}");
        if !r.security.is_admin {
            println!("Hint           : Re-run `omvix-cli.exe probe --verbose` as Administrator to inspect live WMI BIOS responses.");
        }
    }

    println!("\n--- Companion / Potential Conflict Services ---");
    if r.conflict_services.is_empty() {
        println!("None detected.");
    } else {
        for svc in &r.conflict_services {
            println!(
                "  - {:<30} [{:<7}] ({}) {}",
                svc.service_name, svc.status, svc.category, svc.display_name
            );
        }
    }

    println!("\n--- Read-Only WMI BIOS Probe Results ({}) ---", r.probes.len());
    if r.probes.is_empty() {
        println!("  No WMI BIOS probes executed (see WMI Status above).");
    } else {
        println!(
            "{:<28} {:<8} {:<6} {:<6} {:<6} {}",
            "Probe", "Cmd", "Type", "Out", "RC", "Data / Note"
        );
        println!("{}", "-".repeat(92));
        for p in &r.probes {
            let rc_str = p
                .return_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let detail = if p.success {
                p.data_hex.clone()
            } else {
                format!(
                    "{} {}",
                    p.error.as_deref().unwrap_or("Failed"),
                    if p.data_hex.is_empty() {
                        String::new()
                    } else {
                        format!("[{}]", p.data_hex)
                    }
                )
            };
            println!(
                "{:<28} 0x{:<6X} 0x{:<4X} {:<6} {:<6} {}",
                p.name, p.command, p.command_type, p.output_size, rc_str, detail
            );
        }
    }
}
