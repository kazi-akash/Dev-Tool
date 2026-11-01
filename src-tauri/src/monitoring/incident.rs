//! Performance incident recorder. Pure state machine: time and samples are passed in.
//!
//! A short rolling buffer (default 30 s) always holds the most recent samples. When conditions get
//! severe the recorder enters INCIDENT mode, prepends that buffer, keeps recording until the system
//! has been calm for the post-incident window (default 60 s) or the maximum duration is hit, and
//! then emits one `IncidentSummary` with peaks, contributors, a hedged diagnosis and evidence.
//!
//! Disk saturation alone is common during builds and antivirus scans, so it needs to be sustained
//! much longer than a memory trigger before it counts as an incident.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::pressure::PressureState;
use super::telemetry::{
    COMMIT_EXHAUSTED, DISK_SATURATED, MechanismKind, PAGING_HEAVY, classify,
};

pub const PRE_BUFFER_SECS: f64 = 30.0;
pub const POST_SECS: f64 = 60.0;
pub const MAX_INCIDENT_SECS: f64 = 15.0 * 60.0;
/// Memory triggers must hold this long (filters one-sample blips).
const MEMORY_SUSTAIN_SECS: f64 = 3.0;
/// Disk-only trigger must hold this long.
const DISK_SUSTAIN_SECS: f64 = 15.0;
/// After an incident ends, wait before starting another one.
const COOLDOWN_SECS: f64 = 30.0;
/// Hard cap on stored samples per incident (memory bound even at a 1 s interval).
const MAX_SAMPLES: usize = 1_200;
const TIMELINE_POINTS: usize = 120;
const TOP_CONTRIBUTORS: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedBytes {
    pub name: String,
    pub bytes: u64,
}

/// One recorded moment. Optional counters stay `None` when Windows could not provide them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSample {
    pub ts_ms: u64,
    pub ram_percent: f32,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub commit_bytes: Option<u64>,
    pub commit_percent: Option<f32>,
    pub pagefile_percent: Option<f32>,
    pub hard_faults_per_sec: Option<f32>,
    pub cpu_percent: f32,
    pub disk_active_percent: Option<f32>,
    pub pressure: PressureState,
    pub top_processes: Vec<NamedBytes>,
    /// Developer workload states (Docker/WSL2, Android, Node ...).
    pub workloads: Vec<NamedBytes>,
    /// Docker/WSL2 workload memory, when present.
    pub docker_wsl_bytes: Option<u64>,
    /// Build sessions that were active (filled by the build session tracker).
    pub active_sessions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub ts_ms: u64,
    pub ram_percent: f32,
    pub commit_percent: Option<f32>,
    pub hard_faults_per_sec: Option<f32>,
    pub disk_active_percent: Option<f32>,
    pub cpu_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSummary {
    /// Database id; 0 until stored.
    pub id: i64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_secs: u64,
    pub triggers: Vec<String>,
    pub peak_ram_percent: f32,
    pub peak_ram_used_bytes: u64,
    pub min_available_bytes: u64,
    pub peak_commit_bytes: Option<u64>,
    pub peak_commit_percent: Option<f32>,
    pub peak_pagefile_percent: Option<f32>,
    pub peak_hard_faults_per_sec: Option<f32>,
    pub peak_disk_percent: Option<f32>,
    pub peak_cpu_percent: f32,
    /// Highest memory each workload reached during the incident, largest first.
    pub contributors: Vec<NamedBytes>,
    pub sessions: Vec<String>,
    pub mechanism: MechanismKind,
    pub confidence: f32,
    pub diagnosis: String,
    pub evidence: Vec<String>,
    pub timeline: Vec<TimelinePoint>,
}

/// Trigger evaluation for one sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Triggers {
    ram: bool,
    commit: bool,
    faults: bool,
    disk: bool,
    escalation: bool,
}

impl Triggers {
    fn memory(&self) -> bool {
        self.ram || self.commit || self.faults || self.escalation
    }
    fn any(&self) -> bool {
        self.memory() || self.disk
    }
    fn names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.ram {
            v.push("RAM above the critical threshold");
        }
        if self.commit {
            v.push("commit charge near its limit");
        }
        if self.faults {
            v.push("sustained hard-fault spike");
        }
        if self.disk {
            v.push("disk active time saturated");
        }
        if self.escalation {
            v.push("severe memory escalation");
        }
        v
    }
}

fn triggers(s: &IncidentSample, escalating: bool) -> Triggers {
    Triggers {
        ram: s.pressure == PressureState::Critical,
        commit: s.commit_percent.is_some_and(|c| c >= COMMIT_EXHAUSTED),
        faults: s.hard_faults_per_sec.is_some_and(|f| f >= PAGING_HEAVY),
        disk: s.disk_active_percent.is_some_and(|d| d >= DISK_SATURATED),
        // Escalating while already HIGH: climbing fast toward exhaustion.
        escalation: escalating && s.pressure >= PressureState::High,
    }
}

struct Active {
    samples: Vec<IncidentSample>,
    start_secs: f64,
    last_trigger_secs: f64,
    triggers: Vec<&'static str>,
    dropped: usize,
}

pub struct IncidentRecorder {
    pre: VecDeque<(f64, IncidentSample)>,
    active: Option<Active>,
    memory_since: Option<f64>,
    disk_since: Option<f64>,
    cooldown_until: f64,
    pre_secs: f64,
    post_secs: f64,
    max_secs: f64,
}

impl Default for IncidentRecorder {
    fn default() -> Self {
        Self::new(PRE_BUFFER_SECS, POST_SECS, MAX_INCIDENT_SECS)
    }
}

impl IncidentRecorder {
    pub fn new(pre_secs: f64, post_secs: f64, max_secs: f64) -> Self {
        Self {
            pre: VecDeque::new(),
            active: None,
            memory_since: None,
            disk_since: None,
            cooldown_until: 0.0,
            pre_secs,
            post_secs,
            max_secs,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Start time of the running incident, if any.
    pub fn active_since_ms(&self) -> Option<u64> {
        self.active
            .as_ref()
            .and_then(|a| a.samples.first())
            .map(|s| s.ts_ms)
    }

    /// Feed one sample. Returns a summary when an incident just ended.
    pub fn observe(
        &mut self,
        now_secs: f64,
        sample: IncidentSample,
        escalating: bool,
    ) -> Option<IncidentSummary> {
        let t = triggers(&sample, escalating);
        self.memory_since = match (t.memory(), self.memory_since) {
            (true, None) => Some(now_secs),
            (true, s) => s,
            (false, _) => None,
        };
        self.disk_since = match (t.disk, self.disk_since) {
            (true, None) => Some(now_secs),
            (true, s) => s,
            (false, _) => None,
        };
        let memory_sustained = self
            .memory_since
            .is_some_and(|s| now_secs - s >= MEMORY_SUSTAIN_SECS);
        let disk_sustained = self
            .disk_since
            .is_some_and(|s| now_secs - s >= DISK_SUSTAIN_SECS);
        let should_trigger = memory_sustained || disk_sustained;

        if self.active.is_none() {
            self.pre.push_back((now_secs, sample.clone()));
            while self
                .pre
                .front()
                .is_some_and(|(ts, _)| now_secs - ts > self.pre_secs)
            {
                self.pre.pop_front();
            }
            if should_trigger && now_secs >= self.cooldown_until {
                let start_secs = self.pre.front().map_or(now_secs, |(ts, _)| *ts);
                let mut samples: Vec<IncidentSample> =
                    self.pre.drain(..).map(|(_, s)| s).collect();
                if samples.is_empty() {
                    samples.push(sample);
                }
                self.active = Some(Active {
                    samples,
                    start_secs,
                    last_trigger_secs: now_secs,
                    triggers: Vec::new(),
                    dropped: 0,
                });
                if let Some(a) = self.active.as_mut() {
                    add_triggers(a, &t);
                }
            }
            return None;
        }

        let a = self.active.as_mut()?;
        if a.samples.len() >= MAX_SAMPLES {
            // Keep the first quarter (pre-incident context) and drop the oldest of the rest.
            a.samples.remove(a.samples.len() / 4);
            a.dropped += 1;
        }
        a.samples.push(sample);
        if t.any() {
            a.last_trigger_secs = now_secs;
            add_triggers(a, &t);
        }
        let calm_for = now_secs - a.last_trigger_secs;
        let duration = now_secs - a.start_secs;
        if calm_for >= self.post_secs || duration >= self.max_secs {
            let finished = self.active.take()?;
            self.cooldown_until = now_secs + COOLDOWN_SECS;
            self.memory_since = None;
            self.disk_since = None;
            self.pre.clear();
            return Some(summarize(finished.samples, finished.triggers));
        }
        None
    }
}

fn add_triggers(a: &mut Active, t: &Triggers) {
    for n in t.names() {
        if !a.triggers.contains(&n) {
            a.triggers.push(n);
        }
    }
}

fn max_f(values: impl Iterator<Item = Option<f32>>) -> Option<f32> {
    values.flatten().fold(None, |m, v| match m {
        Some(x) if x >= v => Some(x),
        _ => Some(v),
    })
}

/// Build the summary. Public for tests; the recorder is the only production caller.
pub fn summarize(samples: Vec<IncidentSample>, triggers: Vec<&'static str>) -> IncidentSummary {
    let start_ms = samples.first().map_or(0, |s| s.ts_ms);
    let end_ms = samples.last().map_or(start_ms, |s| s.ts_ms);
    let peak_ram_percent = samples.iter().map(|s| s.ram_percent).fold(0.0, f32::max);
    let peak_ram_used_bytes = samples.iter().map(|s| s.used_bytes).max().unwrap_or(0);
    let min_available_bytes = samples.iter().map(|s| s.available_bytes).min().unwrap_or(0);
    let peak_commit_bytes = samples.iter().filter_map(|s| s.commit_bytes).max();
    let peak_commit_percent = max_f(samples.iter().map(|s| s.commit_percent));
    let peak_pagefile_percent = max_f(samples.iter().map(|s| s.pagefile_percent));
    let peak_faults = max_f(samples.iter().map(|s| s.hard_faults_per_sec));
    let peak_disk = max_f(samples.iter().map(|s| s.disk_active_percent));
    let peak_cpu = samples.iter().map(|s| s.cpu_percent).fold(0.0, f32::max);

    // Peak memory per workload; one workload counted once, never summed across samples.
    let mut peaks: HashMap<&str, u64> = HashMap::new();
    for s in &samples {
        for w in &s.workloads {
            let e = peaks.entry(w.name.as_str()).or_insert(0);
            *e = (*e).max(w.bytes);
        }
    }
    let mut contributors: Vec<NamedBytes> = peaks
        .into_iter()
        .map(|(n, b)| NamedBytes {
            name: n.to_string(),
            bytes: b,
        })
        .collect();
    contributors.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    contributors.truncate(TOP_CONTRIBUTORS);

    let mut sessions: Vec<String> = Vec::new();
    for s in &samples {
        for n in &s.active_sessions {
            if !sessions.contains(n) {
                sessions.push(n.clone());
            }
        }
    }

    // Classify with the peak values: the question is what the worst moment looked like.
    let mech = classify(
        PressureState::Critical,
        peak_ram_percent,
        peak_commit_percent,
        peak_faults,
        peak_disk,
    );
    let (diagnosis, confidence) = describe(&mech.kind, &mech, peak_ram_percent);

    let timeline = downsample(&samples);
    IncidentSummary {
        id: 0,
        start_ms,
        end_ms,
        duration_secs: end_ms.saturating_sub(start_ms) / 1000,
        triggers: triggers.iter().map(|t| t.to_string()).collect(),
        peak_ram_percent,
        peak_ram_used_bytes,
        min_available_bytes,
        peak_commit_bytes,
        peak_commit_percent,
        peak_pagefile_percent,
        peak_hard_faults_per_sec: peak_faults,
        peak_disk_percent: peak_disk,
        peak_cpu_percent: peak_cpu,
        contributors,
        sessions,
        mechanism: mech.kind,
        confidence,
        diagnosis,
        evidence: mech.evidence,
        timeline,
    }
}

fn describe(
    kind: &MechanismKind,
    mech: &super::telemetry::MemoryMechanism,
    peak_ram: f32,
) -> (String, f32) {
    let text = match kind {
        MechanismKind::HeavyPaging if mech.disk_saturated || mech.commit_exhausted => {
            "Memory exhaustion caused heavy paging, which saturated the disk and likely caused the system responsiveness problem."
                .to_string()
        }
        MechanismKind::HeavyPaging => {
            "Memory pressure led to paging that appears to have slowed the system. Disk and commit did not reach saturation."
                .to_string()
        }
        MechanismKind::CommitExhaustion => {
            "Commit charge approached its limit. Windows may have refused memory allocations, which can freeze or crash applications even before RAM runs out."
                .to_string()
        }
        MechanismKind::DiskSaturation => {
            "The disk was saturated without matching paging activity. Another workload was probably using the disk; memory is unlikely to be the main cause."
                .to_string()
        }
        _ => format!(
            "RAM reached {peak_ram:.0}% but no heavy paging, commit exhaustion or disk saturation was measured. The cause of the slowdown could not be determined from the available counters."
        ),
    };
    (text, mech.confidence)
}

fn downsample(samples: &[IncidentSample]) -> Vec<TimelinePoint> {
    let step = samples.len().div_ceil(TIMELINE_POINTS).max(1);
    samples
        .iter()
        .step_by(step)
        .map(|s| TimelinePoint {
            ts_ms: s.ts_ms,
            ram_percent: s.ram_percent,
            commit_percent: s.commit_percent,
            hard_faults_per_sec: s.hard_faults_per_sec,
            disk_active_percent: s.disk_active_percent,
            cpu_percent: s.cpu_percent,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn sample(ts: u64, ram: f32, state: PressureState) -> IncidentSample {
        IncidentSample {
            ts_ms: ts * 1000,
            ram_percent: ram,
            available_bytes: ((100.0 - ram) / 100.0 * 16.0 * GB as f32) as u64,
            used_bytes: (ram / 100.0 * 16.0 * GB as f32) as u64,
            commit_bytes: None,
            commit_percent: None,
            pagefile_percent: None,
            hard_faults_per_sec: None,
            cpu_percent: 20.0,
            disk_active_percent: None,
            pressure: state,
            top_processes: vec![],
            workloads: vec![],
            docker_wsl_bytes: None,
            active_sessions: vec![],
        }
    }

    fn calm(ts: u64) -> IncidentSample {
        sample(ts, 50.0, PressureState::Normal)
    }

    fn severe(ts: u64) -> IncidentSample {
        let mut s = sample(ts, 96.0, PressureState::Critical);
        s.commit_percent = Some(98.0);
        s.hard_faults_per_sec = Some(900.0);
        s.disk_active_percent = Some(100.0);
        s
    }

    /// Feed one sample per second; return the summary if one is produced.
    fn run(r: &mut IncidentRecorder, from: u64, to: u64, f: impl Fn(u64) -> IncidentSample) -> Vec<IncidentSummary> {
        (from..to)
            .filter_map(|t| r.observe(t as f64, f(t), false))
            .collect()
    }

    #[test]
    fn calm_system_never_records() {
        let mut r = IncidentRecorder::default();
        assert!(run(&mut r, 0, 600, calm).is_empty());
        assert!(!r.is_active());
    }

    #[test]
    fn single_blip_is_not_an_incident() {
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 50, calm);
        r.observe(50.0, severe(50), false);
        run(&mut r, 51, 120, calm);
        assert!(!r.is_active());
    }

    #[test]
    fn severe_pressure_enters_incident_with_pre_buffer() {
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 100, calm);
        run(&mut r, 100, 105, severe);
        assert!(r.is_active());
        // Pre-buffer reaches back ~30 s before the incident became sustained.
        let start = r.active_since_ms().unwrap() / 1000;
        assert!((70..=76).contains(&start), "start {start}");
    }

    #[test]
    fn records_until_calm_for_post_window_then_summarizes() {
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 100, calm);
        run(&mut r, 100, 140, severe);
        // Still recording through the post window.
        assert!(run(&mut r, 140, 190, calm).is_empty());
        assert!(r.is_active());
        let done = run(&mut r, 190, 205, calm);
        assert_eq!(done.len(), 1);
        let s = &done[0];
        assert!(!r.is_active());
        assert_eq!(s.peak_ram_percent, 96.0);
        assert_eq!(s.peak_commit_percent, Some(98.0));
        assert_eq!(s.peak_disk_percent, Some(100.0));
        assert_eq!(s.peak_hard_faults_per_sec, Some(900.0));
        assert_eq!(s.mechanism, MechanismKind::HeavyPaging);
        assert!(s.diagnosis.contains("heavy paging"), "{}", s.diagnosis);
        assert!(s.diagnosis.contains("likely"));
        assert!(s.confidence >= 0.85);
        assert!(s.triggers.iter().any(|t| t.contains("critical threshold")));
        // ~30 s pre + 40 s severe + 60 s post
        assert!((120..=135).contains(&s.duration_secs), "{}", s.duration_secs);
    }

    #[test]
    fn recurrence_inside_post_window_extends_same_incident() {
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 20, calm);
        run(&mut r, 20, 40, severe);
        run(&mut r, 40, 80, calm);
        let done = run(&mut r, 80, 100, severe);
        assert!(done.is_empty());
        let done = run(&mut r, 100, 170, calm);
        assert_eq!(done.len(), 1, "one incident, not two");
    }

    #[test]
    fn max_duration_ends_a_never_ending_incident() {
        let mut r = IncidentRecorder::new(30.0, 60.0, 120.0);
        let done = run(&mut r, 0, 400, severe);
        assert!(!done.is_empty());
        assert!(done[0].duration_secs <= 125);
    }

    #[test]
    fn cooldown_prevents_instant_restart() {
        let mut r = IncidentRecorder::new(30.0, 60.0, 60.0);
        let mut ended_at = None;
        for t in 0..300u64 {
            if r.observe(t as f64, severe(t), false).is_some() && ended_at.is_none() {
                ended_at = Some(t);
                continue;
            }
            if let Some(e) = ended_at.filter(|e| t <= e + 25) {
                assert!(!r.is_active(), "restarted {} s after the end", t - e);
            }
        }
        assert!(ended_at.is_some());
    }

    #[test]
    fn disk_alone_needs_to_be_sustained_longer() {
        let disk_only = |t| {
            let mut s = calm(t);
            s.disk_active_percent = Some(100.0);
            s
        };
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 10, disk_only);
        assert!(!r.is_active(), "10 s of disk saturation is not an incident");
        run(&mut r, 10, 20, disk_only);
        assert!(r.is_active());
    }

    #[test]
    fn disk_only_incident_does_not_blame_memory() {
        let disk_only = |t| {
            let mut s = calm(t);
            s.disk_active_percent = Some(100.0);
            s.hard_faults_per_sec = Some(2.0);
            s
        };
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 30, disk_only);
        let done = run(&mut r, 30, 100, calm);
        assert_eq!(done.len(), 1);
        // Peak RAM is 50% but the recorder classified under "critical"; disk-only must stay hedged.
        assert_eq!(done[0].mechanism, MechanismKind::DiskSaturation);
        assert!(done[0].diagnosis.contains("probably"));
    }

    #[test]
    fn contributors_use_peak_per_workload_not_a_sum() {
        let mk = |t: u64, g: f64| {
            let mut s = severe(t);
            s.workloads = vec![
                NamedBytes { name: "Gradle".into(), bytes: (g * GB as f64) as u64 },
                NamedBytes { name: "WSL2".into(), bytes: (4.8 * GB as f64) as u64 },
            ];
            s
        };
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 10, |t| mk(t, 1.0));
        run(&mut r, 10, 20, |t| mk(t, 4.2));
        let done = run(&mut r, 20, 100, calm);
        let c = &done[0].contributors;
        assert_eq!(c[0].name, "WSL2");
        assert_eq!(c[1].name, "Gradle");
        assert_eq!(c[1].bytes, (4.2 * GB as f64) as u64);
    }

    #[test]
    fn missing_counters_give_low_confidence_not_certainty() {
        let blind = |t| sample(t, 97.0, PressureState::Critical);
        let mut r = IncidentRecorder::default();
        run(&mut r, 0, 20, blind);
        let done = run(&mut r, 20, 100, calm);
        assert_eq!(done.len(), 1);
        assert!(done[0].confidence <= 0.4);
        assert!(done[0].diagnosis.contains("could not be determined"));
    }

    #[test]
    fn sample_buffer_is_bounded() {
        let mut r = IncidentRecorder::new(30.0, 60.0, 1.0e9);
        for t in 0..5_000u64 {
            r.observe(t as f64, severe(t), false);
        }
        assert!(r.active.as_ref().unwrap().samples.len() <= MAX_SAMPLES);
    }

    #[test]
    fn timeline_is_downsampled() {
        let samples: Vec<_> = (0..1000).map(severe).collect();
        let s = summarize(samples, vec![]);
        assert!(s.timeline.len() <= TIMELINE_POINTS + 1);
        assert!(!s.timeline.is_empty());
    }

    #[test]
    fn summary_serializes_camel_case() {
        let s = summarize(vec![severe(1)], vec!["x"]);
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("peakRamPercent") && j.contains("\"mechanism\":\"HEAVY_PAGING\""), "{j}");
    }
}
