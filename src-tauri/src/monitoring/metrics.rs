//! Summarized system metrics sent to the UI. Pure data + math, no OS calls (unit-testable).

use serde::Serialize;

use super::disks::DiskReport;
use super::pressure::PressureReport;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    /// Unix time in milliseconds when the sample was taken.
    pub timestamp_ms: u64,
    pub memory: MemoryMetrics,
    /// Total CPU usage across all cores, 0-100.
    pub cpu_percent: f32,
    pub disks: Vec<DiskMetrics>,
    /// Free-space state and trend per drive (same order as `disks`).
    pub disk_health: Vec<DiskReport>,
    pub pressure: Option<PressureReport>,
    pub uptime_secs: u64,
    pub process_count: usize,
}

/// `used / total` as a percentage clamped to 0-100. Zero total yields 0 (no NaN/inf to the UI).
pub fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    ((used as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

pub fn memory_metrics(total: u64, available: u64) -> MemoryMetrics {
    let available = available.min(total);
    let used = total - available;
    MemoryMetrics {
        total_bytes: total,
        used_bytes: used,
        available_bytes: available,
        percent: percent(used, total),
    }
}

pub fn disk_metrics(name: String, mount_point: String, total: u64, available: u64) -> DiskMetrics {
    let free = available.min(total);
    let used = total - free;
    DiskMetrics {
        name,
        mount_point,
        total_bytes: total,
        used_bytes: used,
        free_bytes: free,
        used_percent: percent(used, total),
    }
}

/// Sanitize a CPU reading: sysinfo can briefly report values above 100 or NaN.
pub fn clamp_cpu(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn percent_basic() {
        assert_eq!(percent(8 * GB, 16 * GB), 50.0);
        assert_eq!(percent(0, 16 * GB), 0.0);
    }

    #[test]
    fn percent_zero_total_is_zero_not_nan() {
        assert_eq!(percent(5, 0), 0.0);
    }

    #[test]
    fn percent_clamped_when_used_exceeds_total() {
        assert_eq!(percent(20, 10), 100.0);
    }

    #[test]
    fn memory_used_is_total_minus_available() {
        let m = memory_metrics(16 * GB, 4 * GB);
        assert_eq!(m.used_bytes, 12 * GB);
        assert_eq!(m.available_bytes, 4 * GB);
        assert_eq!(m.percent, 75.0);
    }

    #[test]
    fn memory_available_above_total_does_not_underflow() {
        let m = memory_metrics(8 * GB, 9 * GB);
        assert_eq!(m.used_bytes, 0);
        assert_eq!(m.available_bytes, 8 * GB);
    }

    #[test]
    fn disk_free_used_percent() {
        let d = disk_metrics("Windows".into(), "C:\\".into(), 200 * GB, 50 * GB);
        assert_eq!(d.used_bytes, 150 * GB);
        assert_eq!(d.free_bytes, 50 * GB);
        assert_eq!(d.used_percent, 75.0);
    }

    #[test]
    fn disk_zero_total_is_safe() {
        let d = disk_metrics("x".into(), "X:\\".into(), 0, 0);
        assert_eq!(d.used_percent, 0.0);
    }

    #[test]
    fn cpu_clamped_and_nan_safe() {
        assert_eq!(clamp_cpu(150.0), 100.0);
        assert_eq!(clamp_cpu(-3.0), 0.0);
        assert_eq!(clamp_cpu(f32::NAN), 0.0);
        assert_eq!(clamp_cpu(42.5), 42.5);
    }

    #[test]
    fn snapshot_serializes_camel_case() {
        let s = SystemSnapshot {
            timestamp_ms: 1,
            memory: memory_metrics(10, 5),
            cpu_percent: 1.0,
            disks: vec![],
            disk_health: vec![],
            pressure: None,
            uptime_secs: 2,
            process_count: 3,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"timestampMs\":1"));
        assert!(json.contains("\"processCount\":3"));
        assert!(json.contains("\"availableBytes\":5"));
    }
}
