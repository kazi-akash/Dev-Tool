//! RAM pressure state machine. Pure (time is passed in), so every transition is unit-tested.
//!
//! State comes from usage thresholds first, then is adjusted by available memory and commit/pagefile
//! pressure. Trend (falling/stable/rising/escalating) comes from a least-squares slope over recent
//! samples, so "88% and stable" and "88% and climbing fast" are distinguishable.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

const GIB: u64 = 1024 * 1024 * 1024;
/// Dropping to a lower state requires going this many points below the threshold (anti-flapping).
const HYSTERESIS: f32 = 2.0;
/// Samples older than this are discarded.
const HISTORY_SECS: f64 = 600.0;
/// The slope is computed over this recent window.
const TREND_WINDOW_SECS: f64 = 300.0;
const MIN_TREND_SAMPLES: usize = 5;
const MIN_TREND_SPAN_SECS: f64 = 60.0;
/// Percentage points per minute.
const RISING_RATE: f32 = 0.5;
const ESCALATING_RATE: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PressureState {
    Normal,
    Moderate,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Trend {
    Falling,
    Stable,
    Rising,
    Escalating,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PressureThresholds {
    pub moderate: f32,
    pub high: f32,
    pub critical: f32,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            moderate: 70.0,
            high: 85.0,
            critical: 92.0,
        }
    }
}

impl PressureThresholds {
    /// Must satisfy 0 < moderate < high < critical <= 100 with at least 1 point between levels.
    pub fn validated(moderate: f32, high: f32, critical: f32) -> Result<Self, String> {
        let ok = [moderate, high, critical].iter().all(|v| v.is_finite())
            && moderate > 0.0
            && moderate + 1.0 <= high
            && high + 1.0 <= critical
            && critical <= 100.0;
        if ok {
            Ok(Self {
                moderate,
                high,
                critical,
            })
        } else {
            Err(
                "thresholds must satisfy 0 < moderate < high < critical <= 100 (1 point apart)"
                    .into(),
            )
        }
    }

    fn state_for(&self, percent: f32) -> PressureState {
        if percent > self.critical {
            PressureState::Critical
        } else if percent >= self.high {
            PressureState::High
        } else if percent >= self.moderate {
            PressureState::Moderate
        } else {
            PressureState::Normal
        }
    }

    /// Lowest percent at which `state` is entered.
    fn floor_of(&self, state: PressureState) -> f32 {
        match state {
            PressureState::Normal => 0.0,
            PressureState::Moderate => self.moderate,
            PressureState::High => self.high,
            PressureState::Critical => self.critical,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PressureInputs {
    pub percent: f32,
    pub available_bytes: u64,
    pub total_bytes: u64,
    pub commit_percent: Option<f32>,
    pub pagefile_percent: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PressureReport {
    pub state: PressureState,
    pub trend: Trend,
    pub percent: f32,
    pub available_bytes: u64,
    pub commit_percent: Option<f32>,
    pub pagefile_percent: Option<f32>,
    /// Percentage points per minute (positive = rising). 0 until enough samples exist.
    pub rate_per_min: f32,
    /// Minutes until the critical threshold at the current rate, when rising.
    pub minutes_to_critical: Option<f32>,
    /// Human-readable explanation of why this state was chosen.
    pub reasons: Vec<String>,
}

pub struct PressureEngine {
    thresholds: PressureThresholds,
    history: VecDeque<(f64, f32)>,
    state: PressureState,
}

impl Default for PressureEngine {
    fn default() -> Self {
        Self::new(PressureThresholds::default())
    }
}

/// Least-squares slope of percent over time, in points per minute.
fn slope_per_min(samples: &[(f64, f32)]) -> f32 {
    let n = samples.len() as f64;
    let mean_t = samples.iter().map(|s| s.0).sum::<f64>() / n;
    let mean_p = samples.iter().map(|s| s.1 as f64).sum::<f64>() / n;
    let (num, den) = samples.iter().fold((0.0, 0.0), |(num, den), &(t, p)| {
        let dt = t - mean_t;
        (num + dt * (p as f64 - mean_p), den + dt * dt)
    });
    if den == 0.0 {
        0.0
    } else {
        (num / den * 60.0) as f32
    }
}

impl PressureEngine {
    pub fn new(thresholds: PressureThresholds) -> Self {
        Self {
            thresholds,
            history: VecDeque::new(),
            state: PressureState::Normal,
        }
    }

    pub fn thresholds(&self) -> PressureThresholds {
        self.thresholds
    }

    pub fn set_thresholds(&mut self, thresholds: PressureThresholds) {
        self.thresholds = thresholds;
    }

    fn push_sample(&mut self, now: f64, percent: f32) {
        self.history.push_back((now, percent));
        while self
            .history
            .front()
            .is_some_and(|s| now - s.0 > HISTORY_SECS)
        {
            self.history.pop_front();
        }
    }

    fn rate(&self, now: f64) -> f32 {
        let window: Vec<(f64, f32)> = self
            .history
            .iter()
            .copied()
            .filter(|s| now - s.0 <= TREND_WINDOW_SECS)
            .collect();
        let span = window
            .last()
            .zip(window.first())
            .map_or(0.0, |(l, f)| l.0 - f.0);
        if window.len() < MIN_TREND_SAMPLES || span < MIN_TREND_SPAN_SECS {
            return 0.0;
        }
        slope_per_min(&window)
    }

    pub fn update(&mut self, input: PressureInputs, now_secs: f64) -> PressureReport {
        self.push_sample(now_secs, input.percent);
        let rate = self.rate(now_secs);
        let trend = if rate >= ESCALATING_RATE {
            Trend::Escalating
        } else if rate >= RISING_RATE {
            Trend::Rising
        } else if rate <= -RISING_RATE {
            Trend::Falling
        } else {
            Trend::Stable
        };

        let t = self.thresholds;
        let mut reasons = Vec::new();
        let mut state = t.state_for(input.percent);
        reasons.push(format!("RAM usage is {:.0}%", input.percent));

        // Low absolute headroom matters even when the percentage looks fine on a large machine.
        if input.total_bytes > 4 * GIB {
            if input.available_bytes < GIB / 2 && state < PressureState::Critical {
                state = PressureState::Critical;
                reasons.push("less than 512 MB of RAM is available".into());
            } else if input.available_bytes < GIB && state < PressureState::High {
                state = PressureState::High;
                reasons.push("less than 1 GB of RAM is available".into());
            }
        }
        if let Some(commit) = input.commit_percent {
            if commit >= 98.0 && state < PressureState::Critical {
                state = PressureState::Critical;
                reasons.push(format!("commit charge is {commit:.0}% of the limit"));
            } else if commit >= 90.0 && state < PressureState::High {
                state = PressureState::High;
                reasons.push(format!("commit charge is {commit:.0}% of the limit"));
            }
        }
        if let Some(pf) = input.pagefile_percent
            && pf >= 60.0
            && state >= PressureState::High
        {
            reasons.push(format!("pagefile-backed memory is {pf:.0}% used"));
            if state < PressureState::Critical {
                state = PressureState::Critical;
            }
        }

        // Hysteresis: only step down once usage is clearly below the current state's entry point.
        if state < self.state {
            let floor = t.floor_of(self.state);
            if input.percent > floor - HYSTERESIS && input.percent <= floor {
                state = self.state;
                reasons.push(
                    "holding the previous level until usage drops clearly below its threshold"
                        .into(),
                );
            }
        }
        self.state = state;

        match trend {
            Trend::Escalating => {
                reasons.push(format!("usage is rising quickly (+{rate:.1} points/min)"))
            }
            Trend::Rising => reasons.push(format!("usage is rising (+{rate:.1} points/min)")),
            Trend::Falling => reasons.push(format!("usage is falling ({rate:.1} points/min)")),
            Trend::Stable => {}
        }

        let minutes_to_critical = (rate > 0.0 && input.percent <= t.critical)
            .then(|| (t.critical - input.percent) / rate)
            .filter(|m| m.is_finite());

        PressureReport {
            state,
            trend,
            percent: input.percent,
            available_bytes: input.available_bytes,
            commit_percent: input.commit_percent,
            pagefile_percent: input.pagefile_percent,
            rate_per_min: rate,
            minutes_to_critical,
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(percent: f32) -> PressureInputs {
        PressureInputs {
            percent,
            available_bytes: ((100.0 - percent) / 100.0 * 16.0 * GIB as f32) as u64,
            total_bytes: 16 * GIB,
            commit_percent: None,
            pagefile_percent: None,
        }
    }

    fn state_of(percent: f32) -> PressureState {
        PressureEngine::default().update(input(percent), 0.0).state
    }

    #[test]
    fn default_thresholds_map_to_states() {
        assert_eq!(state_of(40.0), PressureState::Normal);
        assert_eq!(state_of(69.9), PressureState::Normal);
        assert_eq!(state_of(70.0), PressureState::Moderate);
        assert_eq!(state_of(84.9), PressureState::Moderate);
        assert_eq!(state_of(85.0), PressureState::High);
        assert_eq!(state_of(92.0), PressureState::High);
        assert_eq!(state_of(92.1), PressureState::Critical);
    }

    #[test]
    fn thresholds_are_configurable_and_validated() {
        let mut e = PressureEngine::new(PressureThresholds::validated(50.0, 60.0, 80.0).unwrap());
        assert_eq!(e.update(input(65.0), 0.0).state, PressureState::High);
        assert!(PressureThresholds::validated(80.0, 70.0, 90.0).is_err());
        assert!(PressureThresholds::validated(70.0, 85.0, 101.0).is_err());
        assert!(PressureThresholds::validated(f32::NAN, 85.0, 92.0).is_err());
        assert!(PressureThresholds::validated(70.0, 70.5, 92.0).is_err());
        assert!(PressureThresholds::validated(0.0, 85.0, 92.0).is_err());
    }

    #[test]
    fn stable_88_percent_is_high_and_stable() {
        let mut e = PressureEngine::default();
        let mut last = None;
        for i in 0..40 {
            last = Some(e.update(input(88.0), i as f64 * 2.0));
        }
        let r = last.unwrap();
        assert_eq!(r.state, PressureState::High);
        assert_eq!(r.trend, Trend::Stable);
        assert_eq!(r.minutes_to_critical, None);
    }

    #[test]
    fn rapidly_climbing_88_percent_is_high_and_escalating() {
        let mut e = PressureEngine::default();
        let mut last = None;
        // 78% -> 88% over 120 s = +5 points/min.
        for i in 0..61 {
            last = Some(e.update(input(78.0 + i as f32 / 6.0), i as f64 * 2.0));
        }
        let r = last.unwrap();
        assert_eq!(r.state, PressureState::High);
        assert_eq!(r.trend, Trend::Escalating);
        assert!(
            r.rate_per_min > 4.0 && r.rate_per_min < 6.0,
            "{}",
            r.rate_per_min
        );
        let eta = r.minutes_to_critical.unwrap();
        assert!(eta > 0.5 && eta < 1.5, "{eta}");
        assert!(r.reasons.iter().any(|s| s.contains("rising quickly")));
    }

    #[test]
    fn slow_climb_is_rising_not_escalating() {
        let mut e = PressureEngine::default();
        let mut last = None;
        for i in 0..61 {
            last = Some(e.update(input(60.0 + i as f32 / 60.0), i as f64 * 2.0)); // +1 point/min
        }
        assert_eq!(last.unwrap().trend, Trend::Rising);
    }

    #[test]
    fn falling_trend_detected() {
        let mut e = PressureEngine::default();
        let mut last = None;
        for i in 0..61 {
            last = Some(e.update(input(80.0 - i as f32 / 30.0), i as f64 * 2.0));
        }
        assert_eq!(last.unwrap().trend, Trend::Falling);
    }

    #[test]
    fn trend_needs_enough_samples_and_span() {
        let mut e = PressureEngine::default();
        e.update(input(50.0), 0.0);
        let r = e.update(input(90.0), 2.0);
        assert_eq!(r.trend, Trend::Stable);
        assert_eq!(r.rate_per_min, 0.0);
    }

    #[test]
    fn heavy_pagefile_use_makes_it_critical() {
        let mut e = PressureEngine::default();
        let mut i = input(88.0);
        i.pagefile_percent = Some(75.0);
        let r = e.update(i, 0.0);
        assert_eq!(r.state, PressureState::Critical);
        assert!(r.reasons.iter().any(|s| s.contains("pagefile")));
    }

    #[test]
    fn pagefile_alone_does_not_escalate_a_healthy_machine() {
        let mut i = input(50.0);
        i.pagefile_percent = Some(90.0);
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::Normal
        );
    }

    #[test]
    fn very_low_available_memory_forces_state() {
        let mut i = input(80.0);
        i.available_bytes = GIB / 4;
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::Critical
        );
        i.available_bytes = GIB * 3 / 4;
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::High
        );
    }

    #[test]
    fn low_available_ignored_on_tiny_machines() {
        let i = PressureInputs {
            percent: 50.0,
            available_bytes: GIB / 4,
            total_bytes: 2 * GIB,
            commit_percent: None,
            pagefile_percent: None,
        };
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::Normal
        );
    }

    #[test]
    fn commit_exhaustion_escalates() {
        let mut i = input(60.0);
        i.commit_percent = Some(99.0);
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::Critical
        );
        i.commit_percent = Some(91.0);
        assert_eq!(
            PressureEngine::default().update(i, 0.0).state,
            PressureState::High
        );
    }

    #[test]
    fn hysteresis_prevents_flapping_at_a_boundary() {
        let mut e = PressureEngine::default();
        assert_eq!(e.update(input(86.0), 0.0).state, PressureState::High);
        // 84.5 is below 85 but within 2 points: hold High.
        assert_eq!(e.update(input(84.5), 2.0).state, PressureState::High);
        // 82.9 is clearly below: step down.
        assert_eq!(e.update(input(82.9), 4.0).state, PressureState::Moderate);
        // Rising again re-enters immediately.
        assert_eq!(e.update(input(85.0), 6.0).state, PressureState::High);
    }

    #[test]
    fn old_samples_are_dropped() {
        let mut e = PressureEngine::default();
        for i in 0..10 {
            e.update(input(50.0), i as f64);
        }
        e.update(input(50.0), 5000.0);
        assert_eq!(e.history.len(), 1);
    }

    #[test]
    fn serializes_screaming_snake_state() {
        let r = PressureEngine::default().update(input(95.0), 0.0);
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"state\":\"CRITICAL\""));
        assert!(j.contains("\"trend\":\"STABLE\""));
        assert!(j.contains("\"minutesToCritical\""));
    }
}
