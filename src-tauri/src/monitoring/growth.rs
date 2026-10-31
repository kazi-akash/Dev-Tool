//! Per-process memory growth detection.
//!
//! Samples process memory at a coarse cadence and flags sustained growth. The result is a
//! *possible* leak with a confidence, never a verdict: caches, indexing and builds also grow.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use crate::processes::model::ProcessInfo;
use crate::util::{fmt_bytes, linear_fit};

/// One sample per process every 30 s.
const SAMPLE_EVERY_SECS: f64 = 30.0;
/// 60 minutes of history per process.
const MAX_SAMPLES: usize = 120;
/// Ignore small processes: growth from 20 MB to 60 MB is not interesting.
const MIN_TRACKED_BYTES: u64 = 100 * 1024 * 1024;
/// Bound memory use of the tracker itself.
const MAX_TRACKED: usize = 60;

const MIN_SPAN_SECS: f64 = 600.0;
const MIN_SAMPLES: usize = 8;
const MIN_GROWTH_BYTES: u64 = 300 * 1024 * 1024;
const MIN_GROWTH_RATIO: f64 = 0.25;
const MIN_MONOTONIC: f64 = 0.75;
const MIN_FIT: f64 = 0.6;
const MIN_CONFIDENCE: f32 = 0.5;
/// A step down smaller than this fraction counts as noise, not a release of memory.
const NOISE_TOLERANCE: f64 = 0.02;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthFinding {
    pub pid: u32,
    pub name: String,
    pub start_bytes: u64,
    pub current_bytes: u64,
    pub growth_bytes: u64,
    pub growth_percent: f32,
    /// Fitted growth rate.
    pub bytes_per_min: f64,
    pub window_minutes: f32,
    /// 0..1. Combines monotonicity, observation length and relative growth.
    pub confidence: f32,
    pub summary: String,
}

/// Analyze one series of `(seconds, bytes)`. Pure.
pub fn analyze(pid: u32, name: &str, samples: &[(f64, u64)]) -> Option<GrowthFinding> {
    if samples.len() < MIN_SAMPLES {
        return None;
    }
    let (first, last) = (samples.first()?, samples.last()?);
    let span = last.0 - first.0;
    if span < MIN_SPAN_SECS || first.1 == 0 {
        return None;
    }
    let growth = last.1.saturating_sub(first.1);
    let ratio = growth as f64 / first.1 as f64;
    if growth < MIN_GROWTH_BYTES || ratio < MIN_GROWTH_RATIO {
        return None;
    }
    // A recent large release (GC, restart of a worker) means this is not sustained growth.
    let peak = samples.iter().map(|s| s.1).max()?;
    if (last.1 as f64) < peak as f64 * 0.9 {
        return None;
    }

    let steps = samples.len() - 1;
    let non_decreasing = samples
        .windows(2)
        .filter(|w| w[1].1 as f64 >= w[0].1 as f64 * (1.0 - NOISE_TOLERANCE))
        .count();
    let mono = non_decreasing as f64 / steps as f64;
    if mono < MIN_MONOTONIC {
        return None;
    }

    let pts: Vec<(f64, f64)> = samples.iter().map(|s| (s.0 / 60.0, s.1 as f64)).collect();
    let (slope, r2) = linear_fit(&pts)?;
    // Sustained growth is roughly linear; a sawtooth that happens to end high is not.
    if slope <= 0.0 || r2 < MIN_FIT {
        return None;
    }

    let confidence =
        (0.35 * mono + 0.25 * (span / 1800.0).min(1.0) + 0.2 * ratio.min(1.0) + 0.2 * r2)
            .clamp(0.0, 0.95) as f32;
    if confidence < MIN_CONFIDENCE {
        return None;
    }

    let minutes = (span / 60.0) as f32;
    Some(GrowthFinding {
        pid,
        name: name.to_string(),
        start_bytes: first.1,
        current_bytes: last.1,
        growth_bytes: growth,
        growth_percent: (ratio * 100.0) as f32,
        bytes_per_min: slope,
        window_minutes: minutes,
        confidence,
        summary: format!(
            "{name} (PID {pid}) grew from {} to {} over {:.0} min. Possible memory growth/leak.",
            fmt_bytes(first.1),
            fmt_bytes(last.1),
            minutes
        ),
    })
}

struct Series {
    name: String,
    samples: VecDeque<(f64, u64)>,
}

#[derive(Default)]
pub struct GrowthTracker {
    series: HashMap<(u32, u64), Series>,
    last_sample_at: Option<f64>,
}

impl GrowthTracker {
    /// Feed the current process list. Cheap to call every tick; it only records every 30 s.
    pub fn observe(&mut self, now_secs: f64, processes: &[ProcessInfo]) {
        if self
            .last_sample_at
            .is_some_and(|t| now_secs - t < SAMPLE_EVERY_SECS)
        {
            return;
        }
        self.last_sample_at = Some(now_secs);

        let mut big: Vec<&ProcessInfo> = processes
            .iter()
            .filter(|p| p.memory_bytes >= MIN_TRACKED_BYTES)
            .collect();
        big.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes));
        big.truncate(MAX_TRACKED);

        let live: std::collections::HashSet<(u32, u64)> =
            big.iter().map(|p| (p.pid, p.start_time)).collect();
        // A process that shrank below the floor or exited loses its history.
        self.series.retain(|k, _| live.contains(k));

        for p in big {
            let s = self
                .series
                .entry((p.pid, p.start_time))
                .or_insert_with(|| Series {
                    name: p.name.clone(),
                    samples: VecDeque::new(),
                });
            s.samples.push_back((now_secs, p.memory_bytes));
            while s.samples.len() > MAX_SAMPLES {
                s.samples.pop_front();
            }
        }
    }

    /// Current findings, largest growth first.
    pub fn findings(&self) -> Vec<GrowthFinding> {
        let mut out: Vec<GrowthFinding> = self
            .series
            .iter()
            .filter_map(|((pid, _), s)| {
                let v: Vec<(f64, u64)> = s.samples.iter().copied().collect();
                analyze(*pid, &s.name, &v)
            })
            .collect();
        out.sort_by_key(|f| std::cmp::Reverse(f.growth_bytes));
        out
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.series.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::model::{Category, sample};

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    fn series(points: &[(f64, u64)]) -> Vec<(f64, u64)> {
        points.to_vec()
    }

    /// minutes -> seconds
    fn m(min: f64) -> f64 {
        min * 60.0
    }

    #[test]
    fn spec_example_node_1_2_to_3_8_gb_is_flagged() {
        // 10:00 1.2 GB, 10:10 1.7, 10:20 2.5, 10:30 3.8 (interpolated every 3 min).
        let pts = [(0.0, 1.2), (10.0, 1.7), (20.0, 2.5), (30.0, 3.8)];
        let mut s = Vec::new();
        for i in 0..=10 {
            let t = i as f64 * 3.0;
            let seg = pts.windows(2).find(|w| t >= w[0].0 && t <= w[1].0).unwrap();
            let f = (t - seg[0].0) / (seg[1].0 - seg[0].0);
            let gb = seg[0].1 + f * (seg[1].1 - seg[0].1);
            s.push((m(t), (gb * GB as f64) as u64));
        }
        let f = analyze(42, "node.exe", &s).expect("should flag");
        assert!(f.growth_bytes > 2 * GB);
        assert!(f.confidence >= 0.5 && f.confidence <= 0.95);
        assert!(f.summary.contains("Possible memory growth/leak"));
        assert!(f.summary.contains("node.exe (PID 42)"));
        assert!(!f.summary.to_lowercase().contains("is leaking"));
        assert!((f.window_minutes - 30.0).abs() < 0.1);
    }

    #[test]
    fn flat_usage_is_not_flagged() {
        let s: Vec<_> = (0..20).map(|i| (m(i as f64), 800 * MB)).collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn noisy_flat_usage_is_not_flagged() {
        let s: Vec<_> = (0..20)
            .map(|i| (m(i as f64), 800 * MB + if i % 2 == 0 { 40 * MB } else { 0 }))
            .collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn sawtooth_gc_pattern_is_not_flagged() {
        let s: Vec<_> = (0..24)
            .map(|i| (m(i as f64), 500 * MB + (i % 6) as u64 * 250 * MB))
            .collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn growth_followed_by_big_release_is_not_flagged() {
        let mut s: Vec<_> = (0..12)
            .map(|i| (m(i as f64), 500 * MB + i as u64 * 100 * MB))
            .collect();
        s.push((m(12.0), 600 * MB));
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn too_short_or_too_few_samples() {
        let short: Vec<_> = (0..9)
            .map(|i| (m(i as f64 * 0.5), 500 * MB + i as u64 * 200 * MB))
            .collect();
        assert!(analyze(1, "x", &short).is_none(), "span under 10 min");
        let few = series(&[(0.0, 500 * MB), (m(15.0), 2 * GB), (m(30.0), 4 * GB)]);
        assert!(analyze(1, "x", &few).is_none(), "under 8 samples");
    }

    #[test]
    fn small_absolute_growth_is_not_flagged() {
        // 100 MB -> 250 MB: +150% but only 150 MB.
        let s: Vec<_> = (0..20)
            .map(|i| (m(i as f64), 100 * MB + i as u64 * 8 * MB))
            .collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn small_relative_growth_is_not_flagged() {
        // 8 GB -> 8.4 GB: +400 MB but only 5%.
        let s: Vec<_> = (0..20)
            .map(|i| (m(i as f64), 8 * GB + i as u64 * 21 * MB))
            .collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn zero_start_does_not_divide_by_zero() {
        let s: Vec<_> = (0..20).map(|i| (m(i as f64), i as u64 * GB / 4)).collect();
        assert!(analyze(1, "x", &s).is_none());
    }

    #[test]
    fn steady_climb_over_longer_window_has_higher_confidence() {
        let short: Vec<_> = (0..11)
            .map(|i| (m(i as f64), 500 * MB + i as u64 * 100 * MB))
            .collect();
        let long: Vec<_> = (0..31)
            .map(|i| (m(i as f64), 500 * MB + i as u64 * 100 * MB))
            .collect();
        let a = analyze(1, "x", &short).unwrap().confidence;
        let b = analyze(1, "x", &long).unwrap().confidence;
        assert!(b > a);
    }

    fn proc_(pid: u32, mb: u64) -> ProcessInfo {
        let mut p = sample(pid, "node.exe", mb * MB, 0.0, Category::Node);
        p.start_time = 1;
        p
    }

    #[test]
    fn tracker_samples_every_30s_and_flags_growth() {
        let mut t = GrowthTracker::default();
        for i in 0..40 {
            let now = i as f64 * 30.0; // 20 minutes
            t.observe(now, &[proc_(7, 500 + i * 60)]);
            // Calls in between are ignored.
            t.observe(now + 10.0, &[proc_(7, 99999)]);
        }
        let f = t.findings();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].pid, 7);
        assert!(
            f[0].current_bytes < 4 * GB,
            "intermediate ticks must not be recorded"
        );
    }

    #[test]
    fn tracker_forgets_exited_and_small_processes_and_caps_size() {
        let mut t = GrowthTracker::default();
        t.observe(0.0, &[proc_(1, 500), proc_(2, 50)]);
        assert_eq!(t.tracked(), 1, "50 MB process is not tracked");
        t.observe(30.0, &[proc_(3, 500)]);
        assert_eq!(t.tracked(), 1, "pid 1 exited");

        let many: Vec<_> = (0..200).map(|i| proc_(i, 200 + i as u64)).collect();
        t.observe(60.0, &many);
        assert_eq!(t.tracked(), MAX_TRACKED);
    }

    #[test]
    fn reused_pid_with_new_start_time_is_a_new_series() {
        let mut t = GrowthTracker::default();
        t.observe(0.0, &[proc_(1, 500)]);
        let mut again = proc_(1, 500);
        again.start_time = 2;
        t.observe(30.0, &[again]);
        assert_eq!(t.tracked(), 1);
        assert_eq!(t.series.values().next().unwrap().samples.len(), 1);
    }
}
