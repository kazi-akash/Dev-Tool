//! Advanced memory telemetry and mechanism classification. Pure: inputs are passed in, so every
//! branch is unit-tested without the OS.
//!
//! RAM percentage alone cannot say *why* a PC is slow. This module combines RAM, commit charge,
//! hard-fault rate and disk activity into one of a few mechanisms and says how sure it is. Values
//! the OS could not provide are `None` and lower the confidence; they are never treated as zero.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::pressure::{PressureReport, PressureState, Trend};
use crate::windows::memory::CommitInfo;
use crate::windows::perf::PerfSample;

/// Smoothing window: counters are averaged over the last N samples so one spike is not an incident.
const WINDOW: usize = 5;
/// Pages read from disk per second. Sustained values above these indicate paging.
pub const PAGING_ELEVATED: f32 = 100.0;
pub const PAGING_HEAVY: f32 = 500.0;
/// Commit charge as a percentage of the commit limit.
pub const COMMIT_HIGH: f32 = 90.0;
pub const COMMIT_EXHAUSTED: f32 = 95.0;
/// Disk busy percentage.
pub const DISK_BUSY: f32 = 80.0;
pub const DISK_SATURATED: f32 = 95.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MechanismKind {
    /// RAM is not under pressure.
    None,
    /// High RAM, no evidence of paging, commit exhaustion or disk saturation.
    HighRamStable,
    HeavyPaging,
    CommitExhaustion,
    DiskSaturation,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMechanism {
    pub kind: MechanismKind,
    /// 0..1. Lower when counters were unavailable or the evidence is indirect.
    pub confidence: f32,
    pub paging: bool,
    pub commit_exhausted: bool,
    pub disk_saturated: bool,
    /// What the user reads. Hedged wording: "appears", "suggests", "probably".
    pub summary: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTelemetry {
    pub physical_total_bytes: u64,
    pub physical_available_bytes: u64,
    pub physical_used_bytes: u64,
    pub commit_bytes: Option<u64>,
    pub commit_limit_bytes: Option<u64>,
    pub commit_percent: Option<f32>,
    /// Real pagefile usage from the performance counter.
    pub pagefile_usage_percent: Option<f32>,
    /// Estimate from commit above RAM (fallback when the counter is missing).
    pub pagefile_estimate_percent: Option<f32>,
    /// Smoothed over the last few samples.
    pub hard_faults_per_sec: Option<f32>,
    pub disk_active_percent: Option<f32>,
    pub trend: Option<Trend>,
    pub mechanism: MemoryMechanism,
}

pub struct TelemetryInputs<'a> {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub commit: Option<CommitInfo>,
    pub perf: PerfSample,
    pub pressure: &'a PressureReport,
}

#[derive(Default)]
pub struct TelemetryEngine {
    faults: VecDeque<f32>,
    disk: VecDeque<f32>,
}

fn push(q: &mut VecDeque<f32>, v: Option<f32>) {
    match v {
        Some(v) => {
            q.push_back(v);
            while q.len() > WINDOW {
                q.pop_front();
            }
        }
        // A missing reading must not keep a stale average alive.
        None => q.clear(),
    }
}

fn mean(q: &VecDeque<f32>) -> Option<f32> {
    (!q.is_empty()).then(|| q.iter().sum::<f32>() / q.len() as f32)
}

impl TelemetryEngine {
    pub fn update(&mut self, i: TelemetryInputs) -> MemoryTelemetry {
        push(&mut self.faults, i.perf.hard_faults_per_sec);
        push(&mut self.disk, i.perf.disk_active_percent);
        let faults = mean(&self.faults);
        let disk = mean(&self.disk);
        let available = i.available_bytes.min(i.total_bytes);
        let commit_percent = i.commit.map(|c| c.commit_percent());
        let mechanism = classify(
            i.pressure.state,
            i.pressure.percent,
            commit_percent,
            faults,
            disk,
        );
        MemoryTelemetry {
            physical_total_bytes: i.total_bytes,
            physical_available_bytes: available,
            physical_used_bytes: i.total_bytes - available,
            commit_bytes: i.commit.map(|c| c.commit_total),
            commit_limit_bytes: i.commit.map(|c| c.commit_limit),
            commit_percent,
            pagefile_usage_percent: i.perf.pagefile_usage_percent,
            pagefile_estimate_percent: i.commit.map(|c| c.pagefile_overflow_percent()),
            hard_faults_per_sec: faults,
            disk_active_percent: disk,
            trend: Some(i.pressure.trend),
            mechanism,
        }
    }
}

fn none_mechanism() -> MemoryMechanism {
    MemoryMechanism {
        kind: MechanismKind::None,
        confidence: 1.0,
        paging: false,
        commit_exhausted: false,
        disk_saturated: false,
        summary: "Memory is not under pressure.".into(),
        evidence: vec![],
    }
}

/// Decide which mechanism explains the memory pressure. `faults`/`disk` are smoothed values.
pub fn classify(
    state: PressureState,
    ram_percent: f32,
    commit_percent: Option<f32>,
    faults: Option<f32>,
    disk: Option<f32>,
) -> MemoryMechanism {
    if state < PressureState::High {
        return none_mechanism();
    }
    let paging = faults.is_some_and(|f| f >= PAGING_ELEVATED);
    let heavy_paging = faults.is_some_and(|f| f >= PAGING_HEAVY);
    let commit_exhausted = commit_percent.is_some_and(|c| c >= COMMIT_EXHAUSTED);
    let commit_high = commit_percent.is_some_and(|c| c >= COMMIT_HIGH);
    let disk_busy = disk.is_some_and(|d| d >= DISK_BUSY);
    let disk_saturated = disk.is_some_and(|d| d >= DISK_SATURATED);

    let mut evidence = vec![format!("RAM usage is {ram_percent:.0}%")];
    match commit_percent {
        Some(c) => evidence.push(format!("commit charge is {c:.0}% of the limit")),
        None => evidence.push("commit charge could not be read".into()),
    }
    match faults {
        Some(f) => evidence.push(format!(
            "{f:.0} pages/s are being read from disk (hard faults)"
        )),
        None => evidence.push("hard-fault rate could not be read".into()),
    }
    match disk {
        Some(d) => evidence.push(format!("disk active time is {d:.0}%")),
        None => evidence.push("disk active time could not be read".into()),
    }

    let missing = [commit_percent.is_none(), faults.is_none(), disk.is_none()]
        .iter()
        .filter(|m| **m)
        .count();
    // Each unavailable signal removes certainty; never below 0.3.
    let penalty = 0.1 * missing as f32;
    let ram = format!("RAM reached {ram_percent:.0}%");

    let (kind, base, summary) = if paging && (disk_busy || commit_high) {
        let mut extra = Vec::new();
        if let Some(c) = commit_percent.filter(|_| commit_high) {
            extra.push(format!("commit usage reached {c:.0}%"));
        }
        if let Some(d) = disk.filter(|_| disk_busy) {
            extra.push(format!("disk activity reached {d:.0}%"));
        }
        (
            MechanismKind::HeavyPaging,
            if heavy_paging || (disk_saturated && commit_high) {
                0.9
            } else {
                0.8
            },
            format!(
                "{ram}, but the main cause of system slowdown appears to be heavy paging. {}, indicating that Windows was heavily using the pagefile.",
                capitalize(&extra.join(" and "))
            ),
        )
    } else if paging {
        (
            MechanismKind::HeavyPaging,
            0.7,
            format!(
                "{ram}, and Windows is reading pages back from disk, which suggests paging is slowing the system. Disk and commit levels do not yet show saturation."
            ),
        )
    } else if commit_exhausted {
        (
            MechanismKind::CommitExhaustion,
            0.75,
            format!(
                "{ram}, and commit charge is near its limit. Windows may start refusing memory allocations, which can crash or freeze applications even before RAM runs out."
            ),
        )
    } else if disk_saturated && faults.is_some() {
        (
            MechanismKind::DiskSaturation,
            0.55,
            format!(
                "{ram} and the disk is saturated, but paging is low. The disk activity is probably not caused by memory pressure; another workload may be using the disk."
            ),
        )
    } else if faults.is_none() && disk.is_none() {
        (
            MechanismKind::HighRamStable,
            0.4,
            format!(
                "{ram}. Paging and disk counters are unavailable, so the slowdown mechanism cannot be determined."
            ),
        )
    } else {
        (
            MechanismKind::HighRamStable,
            0.8,
            format!(
                "{ram}, but there is no sign of heavy paging, commit exhaustion or disk saturation. Memory is high but stable."
            ),
        )
    };

    MemoryMechanism {
        kind,
        confidence: (base - penalty).clamp(0.3, 0.95),
        paging,
        commit_exhausted,
        disk_saturated,
        summary,
        evidence,
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::pressure::{PressureEngine, PressureInputs};

    const GB: u64 = 1024 * 1024 * 1024;

    fn report(percent: f32) -> PressureReport {
        PressureEngine::default().update(
            PressureInputs {
                percent,
                available_bytes: 2 * GB,
                total_bytes: 16 * GB,
                commit_percent: None,
                pagefile_percent: None,
            },
            0.0,
        )
    }

    #[test]
    fn below_high_is_none() {
        let m = classify(
            PressureState::Moderate,
            75.0,
            Some(50.0),
            Some(900.0),
            Some(100.0),
        );
        assert_eq!(m.kind, MechanismKind::None);
    }

    #[test]
    fn high_ram_but_stable() {
        let m = classify(PressureState::High, 88.0, Some(60.0), Some(3.0), Some(8.0));
        assert_eq!(m.kind, MechanismKind::HighRamStable);
        assert!(m.summary.contains("stable"));
        assert!(!m.paging && !m.commit_exhausted && !m.disk_saturated);
        assert!(m.confidence >= 0.75);
    }

    #[test]
    fn heavy_paging_with_commit_and_disk() {
        let m = classify(
            PressureState::Critical,
            96.0,
            Some(98.0),
            Some(900.0),
            Some(100.0),
        );
        assert_eq!(m.kind, MechanismKind::HeavyPaging);
        assert!(m.summary.contains("heavy paging"), "{}", m.summary);
        assert!(
            m.summary.contains("Commit usage reached 98%"),
            "{}",
            m.summary
        );
        assert!(m.summary.contains("disk activity reached 100%"));
        assert!(m.paging && m.commit_exhausted && m.disk_saturated);
        assert!(m.confidence >= 0.85);
    }

    #[test]
    fn paging_without_disk_or_commit_is_hedged() {
        let m = classify(
            PressureState::High,
            90.0,
            Some(70.0),
            Some(250.0),
            Some(30.0),
        );
        assert_eq!(m.kind, MechanismKind::HeavyPaging);
        assert!(m.summary.contains("suggests"));
        assert!(m.confidence < 0.8);
    }

    #[test]
    fn commit_exhaustion_without_paging() {
        let m = classify(
            PressureState::Critical,
            90.0,
            Some(97.0),
            Some(5.0),
            Some(10.0),
        );
        assert_eq!(m.kind, MechanismKind::CommitExhaustion);
        assert!(m.commit_exhausted && !m.paging);
    }

    #[test]
    fn disk_saturation_without_paging_does_not_blame_memory() {
        let m = classify(
            PressureState::High,
            88.0,
            Some(60.0),
            Some(4.0),
            Some(100.0),
        );
        assert_eq!(m.kind, MechanismKind::DiskSaturation);
        assert!(m.summary.contains("probably not caused by memory"));
        assert!(m.confidence <= 0.6);
    }

    #[test]
    fn missing_counters_lower_confidence_and_say_so() {
        let m = classify(PressureState::High, 90.0, None, None, None);
        assert_eq!(m.kind, MechanismKind::HighRamStable);
        assert!(m.summary.contains("cannot be determined"));
        assert!(m.confidence <= 0.4);
        assert!(m.evidence.iter().any(|e| e.contains("could not be read")));
    }

    #[test]
    fn engine_smooths_a_single_spike() {
        let rep = report(90.0);
        let mut e = TelemetryEngine::default();
        let mk = |faults: f32| TelemetryInputs {
            total_bytes: 16 * GB,
            available_bytes: 2 * GB,
            commit: None,
            perf: PerfSample {
                hard_faults_per_sec: Some(faults),
                pagefile_usage_percent: None,
                disk_active_percent: Some(10.0),
            },
            pressure: &rep,
        };
        for _ in 0..4 {
            e.update(mk(0.0));
        }
        let t = e.update(mk(400.0));
        assert!((t.hard_faults_per_sec.unwrap() - 80.0).abs() < 0.01);
        assert!(!t.mechanism.paging, "one spike must not read as paging");
    }

    #[test]
    fn missing_reading_clears_the_average() {
        let mut q = VecDeque::new();
        push(&mut q, Some(500.0));
        push(&mut q, None);
        assert_eq!(mean(&q), None);
    }

    #[test]
    fn telemetry_serializes_and_does_not_underflow() {
        let rep = report(50.0);
        let t = TelemetryEngine::default().update(TelemetryInputs {
            total_bytes: 16 * GB,
            available_bytes: 20 * GB,
            commit: None,
            perf: PerfSample::default(),
            pressure: &rep,
        });
        assert_eq!(t.physical_used_bytes, 0);
        let j = serde_json::to_string(&t).unwrap();
        assert!(
            j.contains("hardFaultsPerSec") && j.contains("\"kind\":\"NONE\""),
            "{j}"
        );
    }
}
