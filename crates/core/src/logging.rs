use crate::error::Result;
use chrono::Local;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer as FmtWriter;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

static PANIC_HOOK_ONCE: Once = Once::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_filter_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    pub fn all() -> &'static [LogLevel] {
        &[
            Self::Error,
            Self::Warn,
            Self::Info,
            Self::Debug,
            Self::Trace,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: LogLevel,
    /// Total maximum disk space across all rotated log files (in MB). Default: 20 MB.
    pub max_total_mb: u64,
    /// Number of rotating log files (`omvix.log` + `omvix.1..N-1.log`). Default: 4 (`4 x 5 MB`).
    pub max_files: usize,
    /// Optional override directory for log files.
    pub log_dir: Option<PathBuf>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            max_total_mb: 20,
            max_files: 4,
            log_dir: None,
        }
    }
}

impl LogConfig {
    pub fn resolved_dir(&self) -> PathBuf {
        self.log_dir.clone().unwrap_or_else(default_log_dir)
    }

    pub fn max_total_bytes(&self) -> u64 {
        self.max_total_mb.max(1) * 1024 * 1024
    }
}

/// Returns the platform-appropriate log directory:
/// - Windows: `%LOCALAPPDATA%\OmvixHub\logs`
/// - Linux: `~/.local/state/omvix-hub/logs` (or XDG data/cache fallback)
pub fn default_log_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("OmvixHub").join("logs");
    }

    if let Some(dirs) = ProjectDirs::from("io", "omvix", "OmvixHub") {
        if let Some(state_dir) = dirs.state_dir() {
            return state_dir.join("logs");
        }
        return dirs.data_local_dir().join("logs");
    }

    std::env::temp_dir().join("omvix-hub").join("logs")
}

struct InnerWriter {
    dir: PathBuf,
    file: BufWriter<File>,
    current_bytes: u64,
    max_bytes_per_file: u64,
    max_files: usize,
}

impl InnerWriter {
    fn open_active(dir: &Path) -> io::Result<(BufWriter<File>, u64)> {
        fs::create_dir_all(dir)?;
        let active_path = dir.join("omvix.log");
        let existing_len = fs::metadata(&active_path).map(|m| m.len()).unwrap_or(0);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)?;
        Ok((BufWriter::new(file), existing_len))
    }

    fn rotate(&mut self) -> io::Result<()> {
        let _ = self.file.flush();

        if self.max_files <= 1 {
            // Single-file reset mode: truncate omvix.log and start fresh.
            let active_path = self.dir.join("omvix.log");
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&active_path)?;
            self.file = BufWriter::new(file);
            self.current_bytes = 0;
            return Ok(());
        }

        // Remove the oldest rotated file if it exists.
        let oldest = self.dir.join(format!("omvix.{}.log", self.max_files - 1));
        if oldest.exists() {
            let _ = fs::remove_file(&oldest);
        }

        // Shift omvix.{i}.log -> omvix.{i+1}.log
        for i in (1..=(self.max_files - 2)).rev() {
            let src = self.dir.join(format!("omvix.{i}.log"));
            let dst = self.dir.join(format!("omvix.{}.log", i + 1));
            if src.exists() {
                let _ = fs::rename(&src, &dst);
            }
        }

        // Move active omvix.log -> omvix.1.log
        let active = self.dir.join("omvix.log");
        let first_backup = self.dir.join("omvix.1.log");
        // Drop handle before renaming on Windows by replacing with a temp open or reopening after rename.
        if active.exists() {
            let dummy = OpenOptions::new()
                .create(true)
                .write(true)
                .open(self.dir.join("omvix.tmp.log"))?;
            self.file = BufWriter::new(dummy);
            let _ = fs::rename(&active, &first_backup);
            let _ = fs::remove_file(self.dir.join("omvix.tmp.log"));
        }

        let (writer, len) = Self::open_active(&self.dir)?;
        self.file = writer;
        self.current_bytes = len;
        Ok(())
    }
}

/// Thread-safe size-bounded ring-rotating file writer.
#[derive(Clone)]
pub struct SizeRotatingWriter {
    inner: Arc<Mutex<InnerWriter>>,
}

impl SizeRotatingWriter {
    pub fn from_config(config: &LogConfig) -> Result<Self> {
        let dir = config.resolved_dir();
        Self::with_byte_limit(dir, config.max_total_bytes(), config.max_files)
    }

    pub fn with_byte_limit(
        dir: impl Into<PathBuf>,
        max_total_bytes: u64,
        max_files: usize,
    ) -> Result<Self> {
        let dir = dir.into();
        let max_files = max_files.max(1);
        let max_bytes_per_file = (max_total_bytes / max_files as u64).max(1024);
        let (file, current_bytes) = InnerWriter::open_active(&dir)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(InnerWriter {
                dir,
                file,
                current_bytes,
                max_bytes_per_file,
                max_files,
            })),
        })
    }

    pub fn flush_all(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            let _ = guard.file.flush();
        }
    }
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("log mutex poisoned"))?;
        if guard.current_bytes > 0
            && guard.current_bytes.saturating_add(buf.len() as u64) > guard.max_bytes_per_file
        {
            guard.rotate()?;
        }
        let written = guard.file.write(buf)?;
        guard.current_bytes = guard.current_bytes.saturating_add(written as u64);
        let _ = guard.file.flush();
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("log mutex poisoned"))?;
        guard.file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SizeRotatingWriter {
    type Writer = SizeRotatingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

struct OmvixLogFormatter;

struct MessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

impl<S, N> FormatEvent<S, N> for OmvixLogFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: FmtWriter<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN ",
            Level::INFO => "INFO ",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        let target = event.metadata().target();

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        write!(writer, "{now} | {level} | {target} | ")?;
        if let Some(msg) = &visitor.message {
            write!(writer, "{msg}")?;
        }
        for field in &visitor.fields {
            if visitor.message.is_some() || !visitor.fields.is_empty() {
                write!(writer, " {field}")?;
            }
        }
        writeln!(writer)
    }
}

#[derive(Clone)]
pub struct LogHandle {
    pub log_dir: PathBuf,
    writer: SizeRotatingWriter,
}

impl LogHandle {
    pub fn flush(&self) {
        self.writer.flush_all();
    }

    pub fn total_bytes(&self) -> u64 {
        total_log_bytes(&self.log_dir)
    }

    pub fn files(&self) -> Vec<PathBuf> {
        list_log_files(&self.log_dir)
    }
}

/// Initializes structured, timestamped, size-bounded logging and installs a panic hook.
pub fn init_logging(config: &LogConfig, log_to_stderr: bool) -> Result<LogHandle> {
    let dir = config.resolved_dir();
    let writer = SizeRotatingWriter::from_config(config)?;
    let filter_str = format!(
        "{},wgpu=warn,wgpu_hal=error,wgpu_core=warn,eframe=warn,egui=warn",
        config.level.as_filter_str()
    );
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .event_format(OmvixLogFormatter)
        .with_writer(writer.clone());

    if log_to_stderr {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .event_format(OmvixLogFormatter)
            .with_writer(std::io::stderr);
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stderr_layer)
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .try_init();
    }

    let panic_writer = writer.clone();
    PANIC_HOOK_ONCE.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let location = panic_info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".to_string());
            let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_string()
            };
            tracing::error!(target: "omvix::panic", location = %location, "PANIC: {payload}");
            panic_writer.flush_all();
            previous_hook(panic_info);
        }));
    });

    Ok(LogHandle {
        log_dir: dir,
        writer,
    })
}

/// Lists Omvix log files in `dir` ordered from active (`omvix.log`) to oldest (`omvix.N.log`).
pub fn list_log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let active = dir.join("omvix.log");
    if active.exists() {
        files.push(active);
    }
    for idx in 1..=32 {
        let p = dir.join(format!("omvix.{idx}.log"));
        if p.exists() {
            files.push(p);
        }
    }
    files
}

/// Computes total disk bytes consumed by Omvix log files in `dir`.
pub fn total_log_bytes(dir: &Path) -> u64 {
    list_log_files(dir)
        .into_iter()
        .filter_map(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

/// Removes all Omvix log files in `dir` and recreates an empty `omvix.log`.
pub fn clear_logs(dir: &Path) -> Result<()> {
    for path in list_log_files(dir) {
        if path.file_name().and_then(|n| n.to_str()) == Some("omvix.log") {
            let _ = OpenOptions::new().write(true).truncate(true).open(&path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// Reads the last `max_lines` lines from `omvix.log` (newest file).
pub fn tail_logs(dir: &Path, max_lines: usize) -> Vec<String> {
    let active = dir.join("omvix.log");
    let Ok(content) = fs::read_to_string(active) else {
        return Vec::new();
    };
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    if lines.len() <= max_lines {
        lines
    } else {
        lines[lines.len() - max_lines..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("omvix_test_{label}_{nanos}"))
    }

    #[test]
    fn rotation_caps_total_bytes_and_preserves_latest_entries() {
        let dir = unique_temp_dir("log_rotation");
        let max_total_bytes = 4 * 1024; // 4 x 1024 = 4096 bytes
        let max_files = 4;
        let mut writer = SizeRotatingWriter::with_byte_limit(&dir, max_total_bytes, max_files)
            .expect("writer created");

        for i in 0..160 {
            let line = format!("2026-11-20 18:30:00.000 | INFO  | test | entry #{i:03} padding-data-0123456789abcdef\n");
            writer.write_all(line.as_bytes()).expect("write line");
        }
        writer.flush().expect("flush");

        let total = total_log_bytes(&dir);
        let files = list_log_files(&dir);
        assert!(
            total <= max_total_bytes,
            "total bytes {total} exceeded cap {max_total_bytes}"
        );
        assert_eq!(files.len(), max_files, "expected {max_files} rotated files");
        assert!(!dir.join("omvix.4.log").exists());

        let tail = tail_logs(&dir, 5);
        assert!(
            tail.iter().any(|l| l.contains("entry #159")),
            "latest entry missing from active log: {tail:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
