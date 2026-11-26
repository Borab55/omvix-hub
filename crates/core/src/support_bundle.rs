use crate::config::AppConfig;
use crate::error::Result;
use crate::logging::list_log_files;
use crate::models::DiagnosticReport;
use chrono::{Datelike, Local, Timelike, Utc};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_MAIN_LOG_BYTES: usize = 512 * 1024;
const MAX_ROTATED_LOG_FILES: usize = 2;

/// Computes standard IEEE 802.3 CRC-32 (`0xEDB88320` reflected polynomial).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn current_dos_datetime() -> (u16, u16) {
    let now = Local::now();
    let year = (now.year() - 1980).clamp(0, 127) as u16;
    let month = (now.month() as u16).clamp(1, 12);
    let day = (now.day() as u16).clamp(1, 31);
    let hour = (now.hour() as u16).min(23);
    let min = (now.minute() as u16).min(59);
    let sec = ((now.second() as u16) / 2).min(29);
    let dos_time = (hour << 11) | (min << 5) | sec;
    let dos_date = (year << 9) | (month << 5) | day;
    (dos_time, dos_date)
}

struct ZipEntryMeta {
    name: String,
    crc: u32,
    size: u32,
    local_offset: u32,
    dos_time: u16,
    dos_date: u16,
}

/// Minimal pure-Rust PKZIP 2.0 STORE (`method = 0`) archive builder requiring zero extra crates.
#[derive(Default)]
pub struct ZipArchiveBuilder {
    buf: Vec<u8>,
    entries: Vec<ZipEntryMeta>,
}

impl ZipArchiveBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, name: &str, data: &[u8]) {
        let normalized = name
            .replace('\\', "/")
            .trim_start_matches('/')
            .trim()
            .to_string();
        if normalized.is_empty() {
            return;
        }
        let name_bytes = normalized.as_bytes();
        let Ok(name_len) = u16::try_from(name_bytes.len()) else {
            return;
        };
        let Ok(data_len) = u32::try_from(data.len()) else {
            return;
        };
        let Ok(local_offset) = u32::try_from(self.buf.len()) else {
            return;
        };

        let crc = crc32(data);
        let (dos_time, dos_date) = current_dos_datetime();

        // Local File Header (30 bytes + filename)
        self.buf.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
        self.buf.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 filename flag
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // STORE
        self.buf.extend_from_slice(&dos_time.to_le_bytes());
        self.buf.extend_from_slice(&dos_date.to_le_bytes());
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf.extend_from_slice(&data_len.to_le_bytes());
        self.buf.extend_from_slice(&data_len.to_le_bytes());
        self.buf.extend_from_slice(&name_len.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.buf.extend_from_slice(name_bytes);
        self.buf.extend_from_slice(data);

        self.entries.push(ZipEntryMeta {
            name: normalized,
            crc,
            size: data_len,
            local_offset,
            dos_time,
            dos_date,
        });
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.buf.len() as u32;
        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            let name_len = name_bytes.len() as u16;
            // Central Directory Header (46 bytes + filename)
            self.buf.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version made by
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
            self.buf.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // STORE
            self.buf.extend_from_slice(&entry.dos_time.to_le_bytes());
            self.buf.extend_from_slice(&entry.dos_date.to_le_bytes());
            self.buf.extend_from_slice(&entry.crc.to_le_bytes());
            self.buf.extend_from_slice(&entry.size.to_le_bytes());
            self.buf.extend_from_slice(&entry.size.to_le_bytes());
            self.buf.extend_from_slice(&name_len.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment len
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk start
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.buf.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.buf
                .extend_from_slice(&entry.local_offset.to_le_bytes());
            self.buf.extend_from_slice(name_bytes);
        }

        let cd_size = (self.buf.len() as u32).saturating_sub(cd_offset);
        let total_entries = u16::try_from(self.entries.len()).unwrap_or(u16::MAX);

        // End of Central Directory Record (22 bytes)
        self.buf.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&total_entries.to_le_bytes());
        self.buf.extend_from_slice(&total_entries.to_le_bytes());
        self.buf.extend_from_slice(&cd_size.to_le_bytes());
        self.buf.extend_from_slice(&cd_offset.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());

        self.buf
    }
}

/// Returns a privacy-sanitized copy of `AppConfig` safe for sharing in support bundles.
pub fn sanitize_config_for_support(cfg: &AppConfig) -> AppConfig {
    let mut clean = cfg.clone();
    if !clean.hotkeys.custom_command.trim().is_empty() {
        clean.hotkeys.custom_command = "<redacted>".to_string();
    }
    if !clean.hotkeys.custom_args.trim().is_empty() {
        clean.hotkeys.custom_args = "<redacted>".to_string();
    }
    clean.logging.log_dir = None;
    clean
}

/// Replaces Windows user profile segments (`X:\Users\<name>`, `X:\\Users\\<name>`, or `X:/Users/<name>`)
/// with `<USER>` to prevent leaking local account names in logs, TOML configs, and JSON diagnostics.
pub fn redact_user_paths(input: &str) -> String {
    let bytes = input.as_bytes();
    if bytes.len() < 9 {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        if idx + 2 < bytes.len() && bytes[idx].is_ascii_alphabetic() && bytes[idx + 1] == b':' {
            let sep1_len = if bytes[idx + 2] == b'\\' {
                if idx + 3 < bytes.len() && bytes[idx + 3] == b'\\' {
                    2
                } else {
                    1
                }
            } else if bytes[idx + 2] == b'/' {
                1
            } else {
                0
            };

            let users_start = idx + 2 + sep1_len;
            if sep1_len > 0
                && users_start + 5 < bytes.len()
                && bytes[users_start..users_start + 5].eq_ignore_ascii_case(b"users")
            {
                let sep2_start = users_start + 5;
                let sep2_len = if bytes[sep2_start] == b'\\' {
                    if sep2_start + 1 < bytes.len() && bytes[sep2_start + 1] == b'\\' {
                        2
                    } else {
                        1
                    }
                } else if bytes[sep2_start] == b'/' {
                    1
                } else {
                    0
                };

                if sep2_len > 0 {
                    let prefix_end = sep2_start + sep2_len;
                    out.push_str(&input[idx..prefix_end]);
                    idx = prefix_end;
                    let rest = &input[idx..];
                    let seg_len = rest
                        .char_indices()
                        .find(|&(_, ch)| {
                            matches!(
                                ch,
                                '\\' | '/'
                                    | '"'
                                    | '\''
                                    | ','
                                    | ';'
                                    | ')'
                                    | ']'
                                    | '>'
                                    | '\r'
                                    | '\n'
                                    | '\t'
                                    | ' '
                            )
                        })
                        .map(|(pos, _)| pos)
                        .unwrap_or(rest.len());
                    if seg_len > 0 {
                        out.push_str("<USER>");
                        idx += seg_len;
                    }
                    continue;
                }
            }
        }

        let ch = input[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
    }

    out
}

/// Reads up to the last `max_bytes` of a UTF-8 file cleanly aligned to character and line boundaries.
pub fn read_tail_utf8(path: &Path, max_bytes: usize) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as usize;
    let truncated = len > max_bytes;
    let offset = len.saturating_sub(max_bytes);
    if offset > 0 {
        file.seek(SeekFrom::Start(offset as u64)).ok()?;
    }
    let mut raw = Vec::with_capacity(len.saturating_sub(offset));
    file.read_to_end(&mut raw).ok()?;

    let mut start = 0usize;
    if truncated {
        while start < raw.len() && (raw[start] & 0b1100_0000) == 0b1000_0000 {
            start += 1;
        }
    }
    let mut text = String::from_utf8_lossy(&raw[start..]).into_owned();
    if truncated {
        if let Some(nl) = text.find('\n') {
            text = format!(
                "...[truncated to last {} KiB]...\n{}",
                max_bytes / 1024,
                &text[nl + 1..]
            );
        }
    }
    Some(text)
}

#[derive(Debug, Clone)]
pub struct SupportBundleInputs {
    pub app_version: String,
    pub config: AppConfig,
    pub diagnostics: Option<DiagnosticReport>,
    pub log_dir: PathBuf,
    pub crash_guard_path: Option<PathBuf>,
    pub extra_text_files: Vec<(String, PathBuf, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportBundleSummary {
    pub output_path: PathBuf,
    pub file_count: usize,
    pub total_bytes: usize,
}

pub fn build_support_bundle_zip(
    out_path: &Path,
    inputs: &SupportBundleInputs,
) -> Result<SupportBundleSummary> {
    let mut zip = ZipArchiveBuilder::new();
    let mut manifest = Vec::new();

    let clean_cfg = sanitize_config_for_support(&inputs.config);
    let cfg_toml = toml::to_string_pretty(&clean_cfg)
        .unwrap_or_else(|e| format!("# Failed serializing config: {e}\n"));
    let cfg_redacted = redact_user_paths(&cfg_toml);
    zip.add_file("config_sanitized.toml", cfg_redacted.as_bytes());
    manifest.push("config_sanitized.toml");

    match &inputs.diagnostics {
        Some(report) => {
            let json = serde_json::to_string_pretty(report)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
            let redacted = redact_user_paths(&json);
            zip.add_file("diagnostics.json", redacted.as_bytes());
            manifest.push("diagnostics.json");
        }
        None => {
            zip.add_file(
                "diagnostics.json",
                b"{\"note\": \"Diagnostic report unavailable at bundle creation time\"}\n",
            );
            manifest.push("diagnostics.json (placeholder)");
        }
    }

    if let Some(cg_path) = &inputs.crash_guard_path {
        if let Some(cg_raw) = read_tail_utf8(cg_path, 64 * 1024) {
            let redacted = redact_user_paths(&cg_raw);
            zip.add_file("crash_guard.json", redacted.as_bytes());
            manifest.push("crash_guard.json");
        }
    }

    let mut added_logs = false;
    for log_path in list_log_files(&inputs.log_dir)
        .into_iter()
        .take(MAX_ROTATED_LOG_FILES)
    {
        if let Some(file_name) = log_path.file_name().and_then(|n| n.to_str()) {
            if let Some(log_text) = read_tail_utf8(&log_path, MAX_MAIN_LOG_BYTES) {
                let redacted = redact_user_paths(&log_text);
                let archive_name = format!("logs/{file_name}");
                zip.add_file(&archive_name, redacted.as_bytes());
                added_logs = true;
            }
        }
    }
    if added_logs {
        manifest.push("logs/*");
    }

    for (archive_name, path, max_bytes) in &inputs.extra_text_files {
        if let Some(text) = read_tail_utf8(path, *max_bytes) {
            let redacted = redact_user_paths(&text);
            zip.add_file(archive_name, redacted.as_bytes());
            manifest.push(archive_name.as_str());
        }
    }

    let readme = format!(
        "Omvix Hub Support Bundle\r\n\
         Version: {}\r\n\
         Generated (UTC): {}\r\n\
         Privacy: User profile directory names (X:\\Users\\...) and custom command targets/args are redacted.\r\n\
         Included artifacts: {}\r\n",
        inputs.app_version,
        Utc::now().to_rfc3339(),
        manifest.join(", ")
    );
    zip.add_file("README.txt", readme.as_bytes());

    let file_count = zip.entry_count();
    let bytes = zip.finish();
    let total_bytes = bytes.len();

    crate::write_bytes_atomically(out_path, &bytes)?;

    Ok(SupportBundleSummary {
        output_path: out_path.to_path_buf(),
        file_count,
        total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("omvix_support_test_{label}_{nanos}"))
    }

    #[test]
    fn crc32_matches_standard_ieee_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    #[test]
    fn zip_archive_builder_produces_valid_pkzip_structure() {
        let mut zip = ZipArchiveBuilder::new();
        zip.add_file("hello.txt", b"Hello Omvix");
        zip.add_file(r"logs\nested.log", b"line1\nline2");
        zip.add_file("", b"ignored");

        assert_eq!(zip.entry_count(), 2);
        let bytes = zip.finish();

        // Starts with Local File Header PK\x03\x04
        assert_eq!(&bytes[0..4], &0x0403_4b50u32.to_le_bytes());

        // Ends with 22-byte End of Central Directory PK\x05\x06
        let eocd_start = bytes.len() - 22;
        assert_eq!(
            &bytes[eocd_start..eocd_start + 4],
            &0x0605_4b50u32.to_le_bytes()
        );
        let entry_count = u16::from_le_bytes([bytes[eocd_start + 10], bytes[eocd_start + 11]]);
        assert_eq!(entry_count, 2);

        let cd_size = u32::from_le_bytes([
            bytes[eocd_start + 12],
            bytes[eocd_start + 13],
            bytes[eocd_start + 14],
            bytes[eocd_start + 15],
        ]) as usize;
        let cd_offset = u32::from_le_bytes([
            bytes[eocd_start + 16],
            bytes[eocd_start + 17],
            bytes[eocd_start + 18],
            bytes[eocd_start + 19],
        ]) as usize;
        assert_eq!(cd_offset + cd_size, eocd_start);
        assert_eq!(
            &bytes[cd_offset..cd_offset + 4],
            &0x0201_4b50u32.to_le_bytes()
        );

        // Check stored filename & payload for first entry
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        assert_eq!(&bytes[30..30 + name_len], b"hello.txt");
        let payload = &bytes[30 + name_len..30 + name_len + 11];
        assert_eq!(payload, b"Hello Omvix");
        let recorded_crc = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(recorded_crc, crc32(b"Hello Omvix"));
    }

    #[test]
    fn sanitize_config_and_redact_user_paths_mask_personal_data() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.custom_command = r"C:\Users\alice\Desktop\SecretTool.exe".to_string();
        cfg.hotkeys.custom_args = "--token=secret123".to_string();
        cfg.logging.log_dir = Some(PathBuf::from(r"C:\Users\alice\logs"));

        let clean = sanitize_config_for_support(&cfg);
        assert_eq!(clean.hotkeys.custom_command, "<redacted>");
        assert_eq!(clean.hotkeys.custom_args, "<redacted>");
        assert!(clean.logging.log_dir.is_none());

        let msg = r#"Opened "C:\Users\borab\AppData\Roaming\OmvixHub\config.toml", JSON "C:\\Users\\borab\\AppData\\Local\\OmvixHub\\log.txt", and D:/Users/Ömer/logs/a.log"#;
        let redacted = redact_user_paths(msg);
        assert!(redacted.contains(r"C:\Users\<USER>\AppData"));
        assert!(redacted.contains(r"C:\\Users\\<USER>\\AppData\\Local\\OmvixHub"));
        assert!(redacted.contains("D:/Users/<USER>/logs"));
        assert!(!redacted.contains("borab"));
        assert!(!redacted.contains("Ömer"));
    }

    #[test]
    fn build_support_bundle_zip_creates_archive_with_sanitized_files_and_truncated_logs() {
        let dir = unique_temp_dir("bundle");
        let log_dir = dir.join("logs");
        fs::create_dir_all(&log_dir).unwrap();

        let big_log = format!(
            "{}LAST_LINE_USER=C:\\Users\\secretname\\Desktop\\test\n",
            "INFO | sample log line 0123456789abcdef\n".repeat(20_000)
        );
        fs::write(log_dir.join("omvix.log"), big_log).unwrap();
        fs::write(
            log_dir.join("omvix.1.log"),
            "rotated log C:\\Users\\secretname\\x\n",
        )
        .unwrap();

        let extra_path = dir.join("svc.log");
        fs::write(&extra_path, "service ok\n").unwrap();

        let out_zip = dir.join("support.zip");
        let summary = build_support_bundle_zip(
            &out_zip,
            &SupportBundleInputs {
                app_version: "0.5.0".to_string(),
                config: AppConfig::default(),
                diagnostics: None,
                log_dir,
                crash_guard_path: None,
                extra_text_files: vec![("logs/svc.log".to_string(), extra_path, 64 * 1024)],
            },
        )
        .expect("build zip");

        assert!(out_zip.exists());
        assert!(!out_zip.with_extension("zip.tmp").exists());
        assert_eq!(summary.file_count, 6); // config, diagnostics placeholder, omvix.log, omvix.1.log, svc.log, README

        let raw_zip = fs::read(&out_zip).unwrap();
        let text_lossy = String::from_utf8_lossy(&raw_zip);
        assert!(text_lossy.contains("config_sanitized.toml"));
        assert!(text_lossy.contains("diagnostics.json"));
        assert!(text_lossy.contains("logs/omvix.log"));
        assert!(text_lossy.contains("logs/omvix.1.log"));
        assert!(text_lossy.contains("logs/svc.log"));
        assert!(text_lossy.contains("README.txt"));
        assert!(text_lossy.contains("C:\\Users\\<USER>\\Desktop\\test"));
        assert!(!text_lossy.contains("logs/*, logs/*"));
        assert!(!text_lossy.contains("secretname"));

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let ps_script = "$z = [System.IO.Compression.ZipFile]::OpenRead($env:OMVIX_ZIP_TEST); try { if ($z.Entries.Count -ne 6) { throw ('bad count ' + $z.Entries.Count) }; foreach ($e in $z.Entries) { $sr = New-Object System.IO.StreamReader($e.Open()); $null = $sr.ReadToEnd(); $sr.Dispose() } } finally { $z.Dispose() }";
            let out = std::process::Command::new(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            )
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("Add-Type -AssemblyName System.IO.Compression.FileSystem; {ps_script}"),
            ])
            .env("OMVIX_ZIP_TEST", &out_zip)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .expect("run PowerShell ZipFile check");
            assert!(
                out.status.success(),
                ".NET ZipFile::OpenRead validation failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
