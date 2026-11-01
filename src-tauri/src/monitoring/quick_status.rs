//! Lightweight status for the tray popup. Pure: derived from the latest cached snapshot and the
//! cached workload list, so it adds no monitoring work (no Docker CLI, no scan, no extra sampling).

use serde::Serialize;

use super::disks::DiskState;
use super::metrics::SystemSnapshot;
use super::pressure::PressureState;
use super::telemetry::MechanismKind;
use crate::developers::workloads::Workload;
use crate::util::fmt_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QuickSeverity {
    Healthy,
    Attention,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickDisk {
    pub mount_point: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub free_percent: f32,
    /// Hours until full at the current rate, when a reliable trend exists.
    pub hours_to_full: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickStatus {
    pub ram_percent: f32,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub cpu_percent: f32,
    /// The drive in the worst state (least free space among equals).
    pub disk: Option<QuickDisk>,
    pub severity: QuickSeverity,
    /// One sentence: what is wrong, or that nothing is.
    pub headline: String,
    pub incident_active: bool,
}

fn from_pressure(s: PressureState) -> QuickSeverity {
    match s {
        PressureState::Normal => QuickSeverity::Healthy,
        PressureState::Moderate => QuickSeverity::Attention,
        PressureState::High => QuickSeverity::High,
        PressureState::Critical => QuickSeverity::Critical,
    }
}

fn from_disk(s: DiskState) -> QuickSeverity {
    match s {
        DiskState::Normal => QuickSeverity::Healthy,
        DiskState::Warning => QuickSeverity::Attention,
        DiskState::High => QuickSeverity::High,
        DiskState::Critical => QuickSeverity::Critical,
    }
}

pub fn quick_status(
    snapshot: &SystemSnapshot,
    workloads: &[Workload],
    incident_active: bool,
) -> QuickStatus {
    let pressure_sev = snapshot
        .pressure
        .as_ref()
        .map_or(QuickSeverity::Healthy, |p| from_pressure(p.state));

    // Worst drive: highest severity, then least free space.
    let worst = snapshot.disk_health.iter().max_by(|a, b| {
        from_disk(a.state).cmp(&from_disk(b.state)).then(
            b.free_percent
                .partial_cmp(&a.free_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let disk_sev = worst.map_or(QuickSeverity::Healthy, |d| from_disk(d.state));
    let severity = pressure_sev.max(disk_sev);

    let ram = snapshot.memory.percent;
    let top = workloads.first();
    let headline = if pressure_sev >= QuickSeverity::High && pressure_sev >= disk_sev {
        let mechanism = snapshot
            .telemetry
            .as_ref()
            .map(|t| &t.mechanism)
            .filter(|m| m.kind != MechanismKind::None);
        match (mechanism, top) {
            (Some(m), _) => m.summary.clone(),
            (None, Some(w)) => format!(
                "RAM is at {ram:.0}%. {} holds the most ({}).",
                w.name,
                fmt_bytes(w.memory_bytes)
            ),
            (None, None) => format!("RAM is at {ram:.0}%."),
        }
    } else if disk_sev >= QuickSeverity::Attention {
        let d = worst.expect("a disk severity above healthy implies a drive");
        let eta = d
            .hours_to_full
            .map(|h| format!(" Full in about {h:.1} h at the current rate."))
            .unwrap_or_default();
        format!(
            "{} has {} free ({:.0}%).{eta}",
            d.mount_point,
            fmt_bytes(d.free_bytes),
            d.free_percent
        )
    } else if pressure_sev == QuickSeverity::Attention {
        format!("RAM is at {ram:.0}%, a little above normal.")
    } else {
        "No problems detected.".to_string()
    };

    QuickStatus {
        ram_percent: ram,
        ram_used_bytes: snapshot.memory.used_bytes,
        ram_total_bytes: snapshot.memory.total_bytes,
        cpu_percent: snapshot.cpu_percent,
        disk: worst.map(|d| QuickDisk {
            mount_point: d.mount_point.clone(),
            free_bytes: d.free_bytes,
            total_bytes: d.total_bytes,
            free_percent: d.free_percent,
            hours_to_full: d.hours_to_full,
        }),
        severity,
        headline,
        incident_active,
    }
}

/// Short text for the tray tooltip, for example `DevPilot · RAM 87% · CPU 12%`.
pub fn tooltip(s: &SystemSnapshot) -> String {
    format!(
        "DevPilot · RAM {:.0}% · CPU {:.0}%",
        s.memory.percent, s.cpu_percent
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::disks::{DiskThresholds, build_report};
    use crate::monitoring::metrics::{disk_metrics, memory_metrics};
    use crate::monitoring::pressure::{PressureEngine, PressureInputs};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn snap(ram_pct: f32, drives: &[(&str, u64, u64)]) -> SystemSnapshot {
        let total = 16 * GIB;
        let used = (total as f64 * ram_pct as f64 / 100.0) as u64;
        let memory = memory_metrics(total, total - used);
        let pressure = PressureEngine::default().update(
            PressureInputs {
                percent: memory.percent,
                available_bytes: memory.available_bytes,
                total_bytes: total,
                commit_percent: None,
                pagefile_percent: None,
            },
            0.0,
        );
        let disks: Vec<_> = drives
            .iter()
            .map(|(m, t, f)| disk_metrics("d".into(), (*m).into(), t * GIB, f * GIB))
            .collect();
        let health = disks
            .iter()
            .map(|d| build_report(&DiskThresholds::default(), d, &[], 0.0))
            .collect();
        SystemSnapshot {
            timestamp_ms: 1,
            memory,
            cpu_percent: 12.4,
            disks,
            disk_health: health,
            pressure: Some(pressure),
            telemetry: None,
            uptime_secs: 1,
            process_count: 1,
        }
    }

    fn workload(name: &str, gb: u64) -> Workload {
        Workload {
            kind: crate::developers::workloads::WorkloadKind::Docker,
            name: name.into(),
            memory_bytes: gb * GIB,
            cpu_percent: 1.0,
            process_count: 1,
            members: vec![],
            top_processes: vec![],
            signals: vec![],
        }
    }

    #[test]
    fn healthy_machine_says_so() {
        let q = quick_status(&snap(40.0, &[("C:\\", 500, 300)]), &[], false);
        assert_eq!(q.severity, QuickSeverity::Healthy);
        assert_eq!(q.headline, "No problems detected.");
        assert_eq!(q.disk.unwrap().mount_point, "C:\\");
    }

    #[test]
    fn high_ram_names_the_largest_workload_when_there_is_no_telemetry() {
        let q = quick_status(
            &snap(90.0, &[("C:\\", 500, 300)]),
            &[workload("Docker / WSL2", 5)],
            false,
        );
        assert_eq!(q.severity, QuickSeverity::High);
        assert!(
            q.headline.contains("Docker / WSL2 holds the most"),
            "{}",
            q.headline
        );
    }

    #[test]
    fn critical_ram_wins_over_a_fuller_disk_headline() {
        let q = quick_status(&snap(97.0, &[("C:\\", 500, 20)]), &[], true);
        assert_eq!(q.severity, QuickSeverity::Critical);
        assert!(q.headline.starts_with("RAM is at 97%"));
        assert!(q.incident_active);
    }

    #[test]
    fn low_disk_is_reported_when_ram_is_fine_and_the_worst_drive_is_picked() {
        let q = quick_status(
            &snap(40.0, &[("C:\\", 500, 300), ("S:\\", 275, 6)]),
            &[],
            false,
        );
        assert!(q.severity >= QuickSeverity::High);
        assert_eq!(q.disk.as_ref().unwrap().mount_point, "S:\\");
        assert!(q.headline.starts_with("S:\\ has"), "{}", q.headline);
    }

    #[test]
    fn moderate_ram_is_attention_not_alarm() {
        let q = quick_status(&snap(75.0, &[("C:\\", 500, 300)]), &[], false);
        assert_eq!(q.severity, QuickSeverity::Attention);
        assert!(q.headline.contains("a little above normal"));
    }

    #[test]
    fn no_drives_and_no_pressure_do_not_panic() {
        let mut s = snap(40.0, &[]);
        s.pressure = None;
        let q = quick_status(&s, &[], false);
        assert_eq!(q.severity, QuickSeverity::Healthy);
        assert!(q.disk.is_none());
    }

    #[test]
    fn tooltip_is_short_and_rounded() {
        assert_eq!(tooltip(&snap(87.4, &[])), "DevPilot · RAM 87% · CPU 12%");
    }

    #[test]
    fn serializes_camel_case() {
        let j = serde_json::to_string(&quick_status(&snap(40.0, &[]), &[], false)).unwrap();
        assert!(
            j.contains("\"ramPercent\"") && j.contains("\"severity\":\"HEALTHY\""),
            "{j}"
        );
    }
}
