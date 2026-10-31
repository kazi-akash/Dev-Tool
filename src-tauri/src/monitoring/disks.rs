//! Disk health: free-space state per drive plus a trend-based prediction of when space runs low.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use super::metrics::DiskMetrics;
use crate::util::linear_fit;

/// One sample per drive per minute, 24 hours kept.
const SAMPLE_EVERY_SECS: f64 = 60.0;
const MAX_SAMPLES: usize = 1440;
/// The trend is fitted over the most recent 6 hours.
const TREND_WINDOW_SECS: f64 = 6.0 * 3600.0;
const MIN_TREND_SPAN_SECS: f64 = 600.0;
const MIN_TREND_SAMPLES: usize = 6;
const MIN_FIT: f64 = 0.6;
/// Ignore drift below 10 MB/h.
const MIN_CONSUMPTION_BYTES_PER_HOUR: f64 = 10.0 * 1024.0 * 1024.0;
/// Predictions further out than this are not useful.
const MAX_PREDICTION_HOURS: f64 = 24.0 * 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiskState {
    Normal,
    Warning,
    High,
    Critical,
}

/// Free-space percentages that start each state: `warning` 20, `high` 10, `critical` 5.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskThresholds {
    pub warning_free: f32,
    pub high_free: f32,
    pub critical_free: f32,
}

impl Default for DiskThresholds {
    fn default() -> Self {
        Self {
            warning_free: 20.0,
            high_free: 10.0,
            critical_free: 5.0,
        }
    }
}

impl DiskThresholds {
    /// Must satisfy 0 < critical < high < warning < 100.
    pub fn validated(
        warning_free: f32,
        high_free: f32,
        critical_free: f32,
    ) -> Result<Self, String> {
        let ok = [warning_free, high_free, critical_free]
            .iter()
            .all(|v| v.is_finite())
            && critical_free > 0.0
            && critical_free < high_free
            && high_free < warning_free
            && warning_free < 100.0;
        if ok {
            Ok(Self {
                warning_free,
                high_free,
                critical_free,
            })
        } else {
            Err("thresholds must satisfy 0 < critical < high < warning < 100 (free space %)".into())
        }
    }

    /// `>warning` Normal, `[high, warning]` Warning, `[critical, high)` High, `<critical` Critical.
    pub fn state_for(&self, free_percent: f32) -> DiskState {
        if free_percent < self.critical_free {
            DiskState::Critical
        } else if free_percent < self.high_free {
            DiskState::High
        } else if free_percent <= self.warning_free {
            DiskState::Warning
        } else {
            DiskState::Normal
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskReport {
    pub mount_point: String,
    pub state: DiskState,
    pub free_percent: f32,
    pub free_bytes: u64,
    pub total_bytes: u64,
    /// Fitted change of free space; negative means the drive is filling up.
    pub trend_bytes_per_hour: Option<f64>,
    /// Hours until free space reaches the critical threshold at the current rate.
    pub hours_to_critical: Option<f64>,
    /// Hours until the drive is full at the current rate.
    pub hours_to_full: Option<f64>,
    /// 0..1, quality of the trend fit. Present with the trend.
    pub trend_confidence: Option<f32>,
    pub reasons: Vec<String>,
}

struct History {
    samples: VecDeque<(f64, u64)>,
}

#[derive(Default)]
pub struct DiskEngine {
    thresholds: DiskThresholds,
    history: HashMap<String, History>,
}

fn free_percent(free: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        ((free as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
    }
}

impl DiskEngine {
    pub fn thresholds(&self) -> DiskThresholds {
        self.thresholds
    }

    pub fn set_thresholds(&mut self, t: DiskThresholds) {
        self.thresholds = t;
    }

    /// Record samples (throttled to one per minute per drive) and build a report per drive.
    pub fn update(&mut self, now_secs: f64, disks: &[DiskMetrics]) -> Vec<DiskReport> {
        let live: std::collections::HashSet<&str> =
            disks.iter().map(|d| d.mount_point.as_str()).collect();
        self.history.retain(|k, _| live.contains(k.as_str()));

        disks
            .iter()
            .map(|d| {
                let h = self
                    .history
                    .entry(d.mount_point.clone())
                    .or_insert_with(|| History {
                        samples: VecDeque::new(),
                    });
                if h.samples
                    .back()
                    .is_none_or(|s| now_secs - s.0 >= SAMPLE_EVERY_SECS)
                {
                    h.samples.push_back((now_secs, d.free_bytes));
                    while h.samples.len() > MAX_SAMPLES {
                        h.samples.pop_front();
                    }
                }
                let samples: Vec<(f64, u64)> = h.samples.iter().copied().collect();
                build_report(&self.thresholds, d, &samples, now_secs)
            })
            .collect()
    }
}

/// Pure: state + trend for one drive from its sample history.
pub fn build_report(
    t: &DiskThresholds,
    d: &DiskMetrics,
    samples: &[(f64, u64)],
    now: f64,
) -> DiskReport {
    let fp = free_percent(d.free_bytes, d.total_bytes);
    let state = t.state_for(fp);
    let mut reasons = vec![format!("{fp:.0}% free")];

    let window: Vec<(f64, f64)> = samples
        .iter()
        .filter(|s| now - s.0 <= TREND_WINDOW_SECS)
        .map(|s| (s.0 / 3600.0, s.1 as f64))
        .collect();
    let span = window
        .last()
        .zip(window.first())
        .map_or(0.0, |(l, f)| (l.0 - f.0) * 3600.0);

    let mut trend = None;
    let mut to_critical = None;
    let mut to_full = None;
    let mut conf = None;

    let enough_history = window.len() >= MIN_TREND_SAMPLES && span >= MIN_TREND_SPAN_SECS;
    let fit = enough_history
        .then(|| linear_fit(&window))
        .flatten()
        .filter(|(_, r2)| *r2 >= MIN_FIT);

    if let Some((slope, r2)) = fit {
        trend = Some(slope);
        conf = Some(r2 as f32);
        if slope < -MIN_CONSUMPTION_BYTES_PER_HOUR {
            let rate = -slope;
            let critical_free = d.total_bytes as f64 * (t.critical_free as f64 / 100.0);
            let until_critical = (d.free_bytes as f64 - critical_free) / rate;
            let until_full = d.free_bytes as f64 / rate;
            if until_critical > 0.0 && until_critical <= MAX_PREDICTION_HOURS {
                to_critical = Some(until_critical);
            }
            if until_full > 0.0 && until_full <= MAX_PREDICTION_HOURS {
                to_full = Some(until_full);
            }
            let per_hour = crate::util::fmt_bytes(rate as u64);
            if let Some(h) = to_critical {
                reasons.push(format!(
                    "filling at {per_hour}/h, critical in about {}",
                    fmt_hours(h)
                ));
            } else if state >= DiskState::Critical {
                reasons.push(format!("filling at {per_hour}/h"));
            }
        }
    }

    DiskReport {
        mount_point: d.mount_point.clone(),
        state,
        free_percent: fp,
        free_bytes: d.free_bytes,
        total_bytes: d.total_bytes,
        trend_bytes_per_hour: trend,
        hours_to_critical: to_critical,
        hours_to_full: to_full,
        trend_confidence: conf,
        reasons,
    }
}

fn fmt_hours(h: f64) -> String {
    if h < 1.0 {
        format!("{:.0} min", h * 60.0)
    } else if h < 48.0 {
        format!("{h:.1} h")
    } else {
        format!("{:.0} days", h / 24.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::metrics::disk_metrics;

    const GB: u64 = 1024 * 1024 * 1024;

    fn drive(total_gb: u64, free_gb: f64) -> DiskMetrics {
        disk_metrics(
            "Win".into(),
            "C:\\".into(),
            total_gb * GB,
            (free_gb * GB as f64) as u64,
        )
    }

    fn state(free_pct: f32) -> DiskState {
        DiskThresholds::default().state_for(free_pct)
    }

    #[test]
    fn default_state_boundaries() {
        assert_eq!(state(50.0), DiskState::Normal);
        assert_eq!(state(20.1), DiskState::Normal);
        assert_eq!(state(20.0), DiskState::Warning);
        assert_eq!(state(10.0), DiskState::Warning);
        assert_eq!(state(9.9), DiskState::High);
        assert_eq!(state(5.0), DiskState::High);
        assert_eq!(state(4.9), DiskState::Critical);
        assert_eq!(state(0.0), DiskState::Critical);
    }

    #[test]
    fn thresholds_validated() {
        assert!(DiskThresholds::validated(30.0, 15.0, 5.0).is_ok());
        assert!(DiskThresholds::validated(10.0, 15.0, 5.0).is_err());
        assert!(DiskThresholds::validated(20.0, 10.0, 0.0).is_err());
        assert!(DiskThresholds::validated(100.0, 10.0, 5.0).is_err());
        assert!(DiskThresholds::validated(f32::NAN, 10.0, 5.0).is_err());
    }

    #[test]
    fn custom_thresholds_change_state() {
        let t = DiskThresholds::validated(40.0, 30.0, 20.0).unwrap();
        assert_eq!(t.state_for(25.0), DiskState::High);
    }

    #[test]
    fn report_state_without_history_has_no_trend() {
        let r = build_report(&DiskThresholds::default(), &drive(200, 10.0), &[], 0.0);
        assert_eq!(r.state, DiskState::High); // 5% free
        assert_eq!(r.trend_bytes_per_hour, None);
        assert_eq!(r.hours_to_critical, None);
    }

    /// Free space falling 5 GB per 15 min => 20 GB/h, sampled each minute.
    fn falling_samples(start_gb: f64, minutes: u32, gb_per_hour: f64) -> Vec<(f64, u64)> {
        (0..=minutes)
            .map(|i| {
                let t = i as f64 * 60.0;
                (
                    t,
                    ((start_gb - gb_per_hour * (i as f64 / 60.0)) * GB as f64) as u64,
                )
            })
            .collect()
    }

    #[test]
    fn predicts_critical_and_full_when_filling() {
        // 500 GB drive, 40 GB free falling 20 GB/h for 45 min -> 25 GB free now.
        let s = falling_samples(40.0, 45, 20.0);
        let d = drive(500, 25.0);
        let r = build_report(&DiskThresholds::default(), &d, &s, 45.0 * 60.0);
        let rate = r.trend_bytes_per_hour.unwrap();
        assert!((rate / GB as f64 + 20.0).abs() < 0.5, "{rate}");
        // critical at 5% of 500 = 25 GB free -> already there: no future prediction for critical.
        assert_eq!(r.hours_to_critical, None);
        let full = r.hours_to_full.unwrap();
        assert!((full - 1.25).abs() < 0.1, "{full}");
        assert!(r.trend_confidence.unwrap() > 0.99);
    }

    #[test]
    fn predicts_time_to_critical_for_a_healthy_but_filling_drive() {
        // 200 GB drive, 60 GB free, falling 10 GB/h. Critical = 10 GB free -> 5 h.
        let s = falling_samples(65.0, 30, 10.0);
        let d = drive(200, 60.0);
        let r = build_report(&DiskThresholds::default(), &d, &s, 30.0 * 60.0);
        let h = r.hours_to_critical.unwrap();
        assert!((h - 5.0).abs() < 0.3, "{h}");
        assert!(r.reasons.iter().any(|x| x.contains("critical in about")));
    }

    #[test]
    fn stable_drive_has_trend_but_no_prediction() {
        let s: Vec<_> = (0..=30).map(|i| (i as f64 * 60.0, 60 * GB)).collect();
        let r = build_report(&DiskThresholds::default(), &drive(200, 60.0), &s, 1800.0);
        assert!(r.trend_bytes_per_hour.unwrap().abs() < 1.0);
        assert_eq!((r.hours_to_critical, r.hours_to_full), (None, None));
    }

    #[test]
    fn growing_free_space_is_not_a_prediction() {
        let s = falling_samples(40.0, 30, -10.0);
        let r = build_report(&DiskThresholds::default(), &drive(200, 45.0), &s, 1800.0);
        assert!(r.trend_bytes_per_hour.unwrap() > 0.0);
        assert_eq!(r.hours_to_full, None);
    }

    #[test]
    fn noisy_history_gives_no_trend() {
        let s: Vec<_> = (0..=30)
            .map(|i| (i as f64 * 60.0, if i % 2 == 0 { 50 * GB } else { 70 * GB }))
            .collect();
        let r = build_report(&DiskThresholds::default(), &drive(200, 60.0), &s, 1800.0);
        assert_eq!(r.trend_bytes_per_hour, None);
    }

    #[test]
    fn too_little_history_gives_no_trend() {
        let s = falling_samples(40.0, 5, 20.0); // 6 samples, 5 minutes
        let r = build_report(&DiskThresholds::default(), &drive(200, 38.0), &s, 300.0);
        assert_eq!(r.trend_bytes_per_hour, None);
    }

    #[test]
    fn slow_drift_below_noise_floor_is_ignored() {
        let s = falling_samples(60.0, 30, 0.001); // ~1 MB/h
        let r = build_report(&DiskThresholds::default(), &drive(200, 60.0), &s, 1800.0);
        assert_eq!(r.hours_to_full, None);
    }

    #[test]
    fn engine_throttles_samples_and_drops_removed_drives() {
        let mut e = DiskEngine::default();
        let d = drive(200, 60.0);
        for i in 0..10 {
            e.update(i as f64 * 10.0, std::slice::from_ref(&d)); // every 10 s
        }
        assert_eq!(e.history["C:\\"].samples.len(), 2, "one per 60 s");
        e.update(100.0, &[]);
        assert!(e.history.is_empty());
    }

    #[test]
    fn engine_reports_state() {
        let mut e = DiskEngine::default();
        let r = e.update(0.0, &[drive(100, 4.0)]);
        assert_eq!(r[0].state, DiskState::Critical);
        assert_eq!(r[0].mount_point, "C:\\");
    }

    #[test]
    fn serializes_for_the_ui() {
        let r = build_report(&DiskThresholds::default(), &drive(100, 4.0), &[], 0.0);
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"state\":\"CRITICAL\""));
        assert!(j.contains("\"hoursToFull\":null"));
    }

    #[test]
    fn spec_example_40_35_28_20_gb() {
        // 40 -> 35 -> 28 -> 20 GB over 45 min on a 200 GB drive (accelerating drain).
        let pts = [(0.0, 40.0), (15.0, 35.0), (30.0, 28.0), (45.0, 20.0)];
        let mut s = Vec::new();
        for i in 0..=45 {
            let t = i as f64;
            let seg = pts.windows(2).find(|w| t >= w[0].0 && t <= w[1].0).unwrap();
            let f = (t - seg[0].0) / (seg[1].0 - seg[0].0);
            s.push((
                t * 60.0,
                ((seg[0].1 + f * (seg[1].1 - seg[0].1)) * GB as f64) as u64,
            ));
        }
        let r = build_report(
            &DiskThresholds::default(),
            &drive(200, 20.0),
            &s,
            45.0 * 60.0,
        );
        assert_eq!(r.state, DiskState::Warning); // 10% free exactly
        let full = r.hours_to_full.expect("prediction expected");
        assert!(full > 0.5 && full < 2.5, "{full}");
    }
}
