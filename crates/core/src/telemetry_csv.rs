use crate::error::Result;
use crate::models::TelemetrySnapshot;
use chrono::{DateTime, Utc};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub const MAX_TELEMETRY_CSV_ROWS: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryCsvRow {
    pub timestamp_utc: DateTime<Utc>,
    pub elapsed_s: f64,
    pub cpu_usage_pct: Option<f32>,
    pub cpu_temp_c: Option<f32>,
    pub chassis_temp_c: Option<f32>,
    pub cpu_power_w: Option<f32>,
    pub gpu_sleeping: bool,
    pub gpu_usage_pct: Option<f32>,
    pub gpu_temp_c: Option<f32>,
    pub gpu_power_w: Option<f32>,
    pub ram_used_gb: Option<f32>,
    pub ram_total_gb: Option<f32>,
    pub fan1_rpm: Option<u32>,
    pub fan2_rpm: Option<u32>,
    pub battery_pct: Option<u8>,
    pub battery_rate_w: Option<f32>,
    pub ac_connected: Option<bool>,
}

impl TelemetryCsvRow {
    pub fn from_snapshot(snap: &TelemetrySnapshot, elapsed_s: f64) -> Self {
        Self {
            timestamp_utc: snap.timestamp.unwrap_or_else(Utc::now),
            elapsed_s,
            cpu_usage_pct: snap.cpu_usage_pct,
            cpu_temp_c: snap.cpu_temp_c,
            chassis_temp_c: snap.chassis_temp_c,
            cpu_power_w: snap.cpu_power_w,
            gpu_sleeping: snap.gpu_sleeping,
            gpu_usage_pct: snap.gpu_usage_pct,
            gpu_temp_c: snap.gpu_temp_c,
            gpu_power_w: snap.gpu_power_w,
            ram_used_gb: snap.ram_used_gb,
            ram_total_gb: snap.ram_total_gb,
            fan1_rpm: snap.fan_rpms.first().copied(),
            fan2_rpm: snap.fan_rpms.get(1).copied(),
            battery_pct: snap.battery_pct,
            battery_rate_w: snap.battery_rate_w,
            ac_connected: snap.ac_connected,
        }
    }
}

fn push_opt_f32(out: &mut String, val: Option<f32>, decimals: usize) {
    if let Some(v) = val.filter(|x| x.is_finite()) {
        let _ = write!(out, "{v:.decimals$}");
    }
}

fn push_opt_disp<T: std::fmt::Display>(out: &mut String, val: Option<T>) {
    if let Some(v) = val {
        let _ = write!(out, "{v}");
    }
}

pub fn format_telemetry_csv(rows: &[TelemetryCsvRow]) -> String {
    let mut out = String::with_capacity(256 + rows.len() * 96);
    out.push_str(
        "timestamp_utc,elapsed_s,cpu_usage_pct,cpu_temp_c,chassis_temp_c,cpu_power_w,gpu_sleeping,gpu_usage_pct,gpu_temp_c,gpu_power_w,ram_used_gb,ram_total_gb,fan1_rpm,fan2_rpm,battery_pct,battery_rate_w,ac_connected\n",
    );

    for r in rows {
        let _ = write!(
            out,
            "{},{:.2},",
            r.timestamp_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            r.elapsed_s
        );
        push_opt_f32(&mut out, r.cpu_usage_pct, 1);
        out.push(',');
        push_opt_f32(&mut out, r.cpu_temp_c, 1);
        out.push(',');
        push_opt_f32(&mut out, r.chassis_temp_c, 1);
        out.push(',');
        push_opt_f32(&mut out, r.cpu_power_w, 2);
        out.push(',');
        out.push_str(if r.gpu_sleeping { "true" } else { "false" });
        out.push(',');
        push_opt_f32(&mut out, r.gpu_usage_pct, 1);
        out.push(',');
        push_opt_f32(&mut out, r.gpu_temp_c, 1);
        out.push(',');
        push_opt_f32(&mut out, r.gpu_power_w, 2);
        out.push(',');
        push_opt_f32(&mut out, r.ram_used_gb, 2);
        out.push(',');
        push_opt_f32(&mut out, r.ram_total_gb, 2);
        out.push(',');
        push_opt_disp(&mut out, r.fan1_rpm);
        out.push(',');
        push_opt_disp(&mut out, r.fan2_rpm);
        out.push(',');
        push_opt_disp(&mut out, r.battery_pct);
        out.push(',');
        push_opt_f32(&mut out, r.battery_rate_w, 2);
        out.push(',');
        push_opt_disp(&mut out, r.ac_connected);
        out.push('\n');
    }

    out
}

pub fn write_telemetry_csv(path: &Path, rows: &[TelemetryCsvRow]) -> Result<usize> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let csv = format_telemetry_csv(rows);
    let tmp = path.with_extension("csv.tmp");
    fs::write(&tmp, csv.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_header_empty_none_cells_and_gpu_sleeping_flag() {
        let mut s1 = TelemetrySnapshot::default();
        s1.timestamp = Some(DateTime::from_timestamp(1_760_000_000, 0).unwrap());
        s1.cpu_usage_pct = Some(12.34);
        s1.cpu_temp_c = None;
        s1.gpu_sleeping = true;
        s1.fan_rpms = vec![2400];
        s1.ac_connected = Some(false);

        let mut s2 = TelemetrySnapshot::default();
        s2.timestamp = Some(DateTime::from_timestamp(1_760_000_001, 0).unwrap());
        s2.cpu_temp_c = Some(64.5);
        s2.gpu_sleeping = false;
        s2.gpu_usage_pct = Some(88.0);
        s2.gpu_temp_c = Some(71.0);
        s2.gpu_power_w = Some(102.345);
        s2.fan_rpms = vec![3800, 3900];
        s2.battery_pct = Some(80);
        s2.ac_connected = Some(true);

        let rows = vec![
            TelemetryCsvRow::from_snapshot(&s1, 1.0),
            TelemetryCsvRow::from_snapshot(&s2, 2.0),
        ];
        let csv = format_telemetry_csv(&rows);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "timestamp_utc,elapsed_s,cpu_usage_pct,cpu_temp_c,chassis_temp_c,cpu_power_w,gpu_sleeping,gpu_usage_pct,gpu_temp_c,gpu_power_w,ram_used_gb,ram_total_gb,fan1_rpm,fan2_rpm,battery_pct,battery_rate_w,ac_connected"
        );
        let cols1: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(cols1.len(), 17);
        assert_eq!(cols1[1], "1.00");
        assert_eq!(cols1[2], "12.3");
        assert_eq!(cols1[3], ""); // missing cpu_temp_c
        assert_eq!(cols1[6], "true"); // gpu_sleeping
        assert_eq!(cols1[12], "2400");
        assert_eq!(cols1[13], ""); // missing fan2_rpm
        assert_eq!(cols1[16], "false");

        let cols2: Vec<&str> = lines[2].split(',').collect();
        assert_eq!(cols2[6], "false");
        assert_eq!(cols2[9], "102.35");
        assert_eq!(cols2[13], "3900");
        assert_eq!(cols2[14], "80");
        assert_eq!(cols2[16], "true");
    }

    #[test]
    fn write_telemetry_csv_writes_atomically_without_tmp_leftover() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omvix_csv_test_{nanos}"));
        let out = dir.join("telemetry.csv");

        let row = TelemetryCsvRow::from_snapshot(&TelemetrySnapshot::default(), 0.5);
        let written = write_telemetry_csv(&out, &[row]).expect("write csv");
        assert_eq!(written, 1);
        assert!(out.exists());
        assert!(!out.with_extension("csv.tmp").exists());
        let content = fs::read_to_string(&out).unwrap();
        assert!(content.starts_with("timestamp_utc,"));

        let _ = fs::remove_dir_all(&dir);
    }
}
