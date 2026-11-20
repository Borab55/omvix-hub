use clap::{Parser, Subcommand};
use omvix_core::{
    AppConfig, DiagnosticReport, DiagnosticsBackend, LogLevel, init_logging, list_log_files,
    total_log_bytes,
};
use omvix_windows::WindowsDiagnosticsBackend;
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
    }
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
