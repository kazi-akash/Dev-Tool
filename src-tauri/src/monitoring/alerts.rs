//! Notification policy: which conditions deserve a toast, and when a toast is a repeat.
//! Pure: time and snapshots are passed in, so every rule is unit-tested without Windows.
//!
//! Principles:
//! * Notify when a condition is *entered*, not while it persists.
//! * A persisting condition is announced once; only a **worse** level (escalation) interrupts that,
//!   and it bypasses the cooldown.
//! * A condition must be gone for a while before it can be announced again, and even then a
//!   cooldown applies, so a value hovering around a threshold cannot spam.
//! * User-initiated results (scan finished, action finished) are skipped while the window is on
//!   screen: the user is already looking at the answer.

use std::collections::HashMap;

use super::metrics::SystemSnapshot;
use super::pressure::{PressureState, Trend};
use super::telemetry::MechanismKind;
use crate::monitoring::disks::DiskState;
use crate::util::fmt_bytes;

/// A condition must be absent this long before the same alert may fire again.
const CLEAR_MS: u64 = 120_000;
const MEMORY_COOLDOWN_MS: u64 = 10 * 60_000;
const DISK_COOLDOWN_MS: u64 = 60 * 60_000;
/// User-initiated results of one kind are spaced at least this far apart.
const RESULT_SPACING_MS: u64 = 5_000;
/// Toast text is short; the full detail lives in the app.
const BODY_MAX: usize = 220;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Memory,
    Disk,
    Incident,
    ScanComplete,
    ActionComplete,
}

/// Where the toast's button (and a click on the toast) leads. Fixed allow-listed routes only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Diagnosis,
    Storage,
    History,
    Actions,
}

impl Destination {
    pub fn route(self) -> &'static str {
        match self {
            Self::Diagnosis => "/diagnosis",
            Self::Storage => "/storage",
            Self::History => "/history",
            Self::Actions => "/actions",
        }
    }
    pub fn button(self) -> &'static str {
        match self {
            Self::Diagnosis => "View diagnosis",
            Self::Storage => "View results",
            Self::History => "View incident",
            Self::Actions => "View actions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// Identity for de-duplication, for example `memory` or `disk:S:\`.
    pub key: String,
    pub kind: AlertKind,
    /// Higher is worse. A higher level than the last one announced bypasses the cooldown.
    pub level: u8,
    pub title: String,
    pub body: String,
    pub destination: Destination,
    /// Result of something the user just did (so it is skipped while the window is visible).
    pub user_initiated: bool,
}

fn clip(s: &str) -> String {
    if s.chars().count() <= BODY_MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(BODY_MAX - 1).collect();
    out.push('…');
    out
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    last_sent_ms: u64,
    level: u8,
    /// The condition is currently present.
    active: bool,
    /// When the condition was last seen present (to decide it has cleared).
    last_seen_ms: u64,
}

#[derive(Default)]
pub struct AlertPolicy {
    entries: HashMap<String, Entry>,
}

impl AlertPolicy {
    /// Memory and disk conditions present in this snapshot. Edge/dedupe filtering happens in
    /// [`AlertPolicy::admit`]; this only says what is true now.
    pub fn conditions(snapshot: &SystemSnapshot, top: Option<(&str, u64)>) -> Vec<Alert> {
        let mut out = Vec::new();

        if let Some(p) = snapshot.pressure.as_ref() {
            let mech = snapshot
                .telemetry
                .as_ref()
                .map(|t| &t.mechanism)
                .filter(|m| m.kind != MechanismKind::None);
            let severe_mechanism = mech.is_some_and(|m| {
                matches!(
                    m.kind,
                    MechanismKind::HeavyPaging | MechanismKind::CommitExhaustion
                )
            });
            let holder = top
                .map(|(n, b)| format!(" {n} holds the most ({}).", fmt_bytes(b)))
                .unwrap_or_default();

            if p.state == PressureState::Critical {
                let (level, title, body) = if severe_mechanism {
                    (
                        3,
                        "Memory exhaustion is slowing your PC".to_string(),
                        mech.map(|m| m.summary.clone()).unwrap_or_default(),
                    )
                } else {
                    (
                        2,
                        format!("RAM is critical ({:.0}%)", p.percent),
                        match mech {
                            Some(m) => m.summary.clone(),
                            None => format!(
                                "Only {} is available.{holder}",
                                fmt_bytes(p.available_bytes)
                            ),
                        },
                    )
                };
                out.push(Alert {
                    key: "memory".into(),
                    kind: AlertKind::Memory,
                    level,
                    title,
                    body: clip(&body),
                    destination: Destination::Diagnosis,
                    user_initiated: false,
                });
            } else if p.trend == Trend::Escalating && p.state >= PressureState::High {
                let eta = p
                    .minutes_to_critical
                    .map(|m| format!(" Critical in about {m:.0} min at this rate."))
                    .unwrap_or_default();
                out.push(Alert {
                    key: "memory".into(),
                    kind: AlertKind::Memory,
                    level: 1,
                    title: "Memory is climbing fast".into(),
                    body: clip(&format!(
                        "RAM is at {:.0}% and rising {:.1} points per minute.{eta}{holder}",
                        p.percent, p.rate_per_min
                    )),
                    destination: Destination::Diagnosis,
                    user_initiated: false,
                });
            }
        }

        for d in snapshot
            .disk_health
            .iter()
            .filter(|d| d.state == DiskState::Critical)
        {
            let eta = d
                .hours_to_full
                .map(|h| format!(" Full in about {h:.1} h at the current rate."))
                .unwrap_or_default();
            out.push(Alert {
                key: format!("disk:{}", d.mount_point),
                kind: AlertKind::Disk,
                level: 2,
                title: format!("Drive {} is almost full", d.mount_point),
                body: clip(&format!(
                    "{} free ({:.0}%).{eta}",
                    fmt_bytes(d.free_bytes),
                    d.free_percent
                )),
                destination: Destination::Diagnosis,
                user_initiated: false,
            });
        }
        out
    }

    fn cooldown(kind: AlertKind) -> u64 {
        match kind {
            AlertKind::Memory => MEMORY_COOLDOWN_MS,
            AlertKind::Disk => DISK_COOLDOWN_MS,
            _ => 0,
        }
    }

    /// Feed the conditions true right now. Returns the alerts to show, and forgets conditions that
    /// have been gone long enough.
    pub fn evaluate(&mut self, now_ms: u64, present: Vec<Alert>) -> Vec<Alert> {
        // Mark what is still present; anything absent for CLEAR_MS becomes inactive.
        let present_keys: Vec<&str> = present.iter().map(|a| a.key.as_str()).collect();
        for (key, e) in self.entries.iter_mut() {
            if e.active
                && !present_keys.contains(&key.as_str())
                && now_ms.saturating_sub(e.last_seen_ms) >= CLEAR_MS
            {
                e.active = false;
            }
        }
        let mut out = Vec::new();
        for a in present {
            if self.admit(&a, now_ms) {
                out.push(a);
            }
        }
        out
    }

    /// Decide for one alert. Updates the bookkeeping when it is admitted.
    pub fn admit(&mut self, a: &Alert, now_ms: u64) -> bool {
        match self.entries.get_mut(&a.key) {
            None => {
                self.entries.insert(
                    a.key.clone(),
                    Entry {
                        last_sent_ms: now_ms,
                        level: a.level,
                        active: true,
                        last_seen_ms: now_ms,
                    },
                );
                true
            }
            Some(e) => {
                e.last_seen_ms = now_ms;
                if e.active {
                    // Same ongoing condition: only a worse level interrupts it.
                    if a.level > e.level {
                        e.level = a.level;
                        e.last_sent_ms = now_ms;
                        return true;
                    }
                    return false;
                }
                // A new episode: needs the cooldown, unless it is worse than the last announcement.
                let cooled = now_ms.saturating_sub(e.last_sent_ms) >= Self::cooldown(a.kind);
                let worse = a.level > e.level;
                e.active = true;
                if cooled || worse {
                    e.level = a.level;
                    e.last_sent_ms = now_ms;
                    true
                } else {
                    // Suppressed, but keep the episode level so it does not re-fire at the same level.
                    e.level = e.level.max(a.level);
                    false
                }
            }
        }
    }

    /// One-off results (scan, action, incident): unique keys, spaced per kind, and skipped for
    /// user-initiated results while the window is visible.
    pub fn admit_result(&mut self, a: &Alert, now_ms: u64, window_visible: bool) -> bool {
        if a.user_initiated && window_visible {
            return false;
        }
        let spacing_key = format!("last:{:?}", a.kind);
        if let Some(e) = self.entries.get(&spacing_key)
            && now_ms.saturating_sub(e.last_sent_ms) < RESULT_SPACING_MS
        {
            return false;
        }
        if self.entries.contains_key(&a.key) {
            return false; // the exact same result was already announced
        }
        for k in [a.key.clone(), spacing_key] {
            self.entries.insert(
                k,
                Entry {
                    last_sent_ms: now_ms,
                    level: a.level,
                    active: false,
                    last_seen_ms: now_ms,
                },
            );
        }
        true
    }
}

pub fn scan_alert(entries: usize, total_bytes: u64, complete: bool, started_at_ms: u64) -> Alert {
    Alert {
        key: format!("scan:{started_at_ms}"),
        kind: AlertKind::ScanComplete,
        level: 0,
        title: if complete {
            "Storage scan finished".into()
        } else {
            "Storage scan stopped early".into()
        },
        body: clip(&format!(
            "{entries} item(s), {} in total{}.",
            fmt_bytes(total_bytes),
            if complete { "" } else { " (partial result)" }
        )),
        destination: Destination::Storage,
        user_initiated: true,
    }
}

pub fn action_alert(record_id: u64, name: &str, ok: bool, summary: &str, automated: bool) -> Alert {
    Alert {
        key: format!("action:{record_id}"),
        kind: AlertKind::ActionComplete,
        level: 0,
        title: if ok {
            format!("{name} finished")
        } else {
            format!("{name} failed")
        },
        body: clip(summary),
        destination: Destination::Actions,
        // Automated runs happen without the user watching, so they always deserve a toast.
        user_initiated: !automated,
    }
}

pub fn incident_alert(start_ms: u64, duration_secs: u64, diagnosis: &str) -> Alert {
    Alert {
        key: format!("incident:{start_ms}"),
        kind: AlertKind::Incident,
        level: 0,
        title: format!("Performance incident recorded ({duration_secs} s)"),
        body: clip(diagnosis),
        destination: Destination::History,
        user_initiated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::disks::{DiskThresholds, build_report};
    use crate::monitoring::metrics::{disk_metrics, memory_metrics};
    use crate::monitoring::pressure::{PressureEngine, PressureInputs};
    use crate::monitoring::telemetry::{TelemetryEngine, TelemetryInputs};
    use crate::windows::perf::PerfSample;

    const GIB: u64 = 1024 * 1024 * 1024;
    const MIN: u64 = 60_000;

    fn snap(pct: f32, free_gb: u64, faults: Option<f32>, disk_busy: Option<f32>) -> SystemSnapshot {
        let total = 16 * GIB;
        let used = (total as f64 * pct as f64 / 100.0) as u64;
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
        let telemetry = TelemetryEngine::default().update(TelemetryInputs {
            total_bytes: total,
            available_bytes: memory.available_bytes,
            commit: None,
            perf: PerfSample {
                hard_faults_per_sec: faults,
                pagefile_usage_percent: None,
                disk_active_percent: disk_busy,
            },
            pressure: &pressure,
        });
        let d = disk_metrics("d".into(), "S:\\".into(), 275 * GIB, free_gb * GIB);
        SystemSnapshot {
            timestamp_ms: 1,
            memory,
            cpu_percent: 1.0,
            disk_health: vec![build_report(&DiskThresholds::default(), &d, &[], 0.0)],
            disks: vec![d],
            pressure: Some(pressure),
            telemetry: Some(telemetry),
            uptime_secs: 1,
            process_count: 1,
        }
    }

    fn conds(s: &SystemSnapshot) -> Vec<Alert> {
        AlertPolicy::conditions(s, Some(("Docker / WSL2", 5 * GIB)))
    }

    #[test]
    fn calm_machine_has_no_conditions() {
        assert!(conds(&snap(50.0, 200, Some(1.0), Some(5.0))).is_empty());
        // High but not critical or escalating: no toast.
        assert!(conds(&snap(88.0, 200, Some(1.0), Some(5.0))).is_empty());
    }

    #[test]
    fn critical_ram_alert_has_a_summary_and_a_view_diagnosis_destination() {
        let a = conds(&snap(97.0, 200, Some(2.0), Some(5.0)));
        let m = a.iter().find(|a| a.kind == AlertKind::Memory).unwrap();
        assert_eq!(m.level, 2);
        assert!(m.title.contains("97%"));
        assert_eq!(m.destination.button(), "View diagnosis");
        assert_eq!(m.destination.route(), "/diagnosis");
        assert!(!m.user_initiated);
    }

    #[test]
    fn heavy_paging_is_a_worse_level_than_plain_critical() {
        let a = conds(&snap(97.0, 200, Some(900.0), Some(100.0)));
        let m = a.iter().find(|a| a.kind == AlertKind::Memory).unwrap();
        assert_eq!(m.level, 3);
        assert!(m.title.contains("slowing"));
        assert!(m.body.contains("heavy paging"), "{}", m.body);
    }

    #[test]
    fn low_disk_alert_names_the_drive_and_free_space() {
        let a = conds(&snap(40.0, 6, Some(1.0), Some(5.0)));
        let d = a.iter().find(|a| a.kind == AlertKind::Disk).unwrap();
        assert_eq!(d.key, "disk:S:\\");
        assert!(d.title.contains("S:\\") && d.body.contains("free"));
    }

    #[test]
    fn a_persisting_condition_is_announced_once() {
        let mut p = AlertPolicy::default();
        let s = snap(97.0, 200, Some(2.0), Some(5.0));
        let mut sent = 0;
        for i in 0..100u64 {
            sent += p.evaluate(i * 2_000, conds(&s)).len();
        }
        assert_eq!(sent, 1, "200 s of constant pressure is one toast");
    }

    #[test]
    fn escalation_bypasses_the_cooldown_exactly_once_per_level() {
        let mut p = AlertPolicy::default();
        let plain = snap(97.0, 200, Some(2.0), Some(5.0));
        let paging = snap(97.0, 200, Some(900.0), Some(100.0));
        assert_eq!(p.evaluate(0, conds(&plain)).len(), 1);
        // Seconds later it gets worse: a second toast despite the 10 minute cooldown.
        let esc = p.evaluate(10_000, conds(&paging));
        assert_eq!(esc.len(), 1);
        assert_eq!(esc[0].level, 3);
        // Flapping between the levels does not re-notify.
        assert!(p.evaluate(20_000, conds(&plain)).is_empty());
        assert!(p.evaluate(30_000, conds(&paging)).is_empty());
    }

    #[test]
    fn a_returning_condition_waits_for_clear_and_cooldown() {
        let mut p = AlertPolicy::default();
        let bad = snap(97.0, 200, Some(2.0), Some(5.0));
        let ok = snap(50.0, 200, Some(1.0), Some(5.0));
        assert_eq!(p.evaluate(0, conds(&bad)).len(), 1);
        // Brief dip below the threshold, then back: still the same episode.
        assert!(p.evaluate(10_000, conds(&ok)).is_empty());
        assert!(
            p.evaluate(20_000, conds(&bad)).is_empty(),
            "episode never cleared"
        );
        // Gone for over two minutes, back within the cooldown: suppressed.
        assert!(p.evaluate(30_000, conds(&ok)).is_empty());
        assert!(p.evaluate(200_000, conds(&ok)).is_empty());
        assert!(
            p.evaluate(5 * MIN, conds(&bad)).is_empty(),
            "inside the 10 minute cooldown"
        );
        // After the cooldown a new episode is announced again.
        assert!(p.evaluate(6 * MIN, conds(&ok)).is_empty());
        assert!(p.evaluate(8 * MIN, conds(&ok)).is_empty());
        assert_eq!(p.evaluate(12 * MIN, conds(&bad)).len(), 1);
    }

    #[test]
    fn a_worse_return_inside_the_cooldown_still_notifies() {
        let mut p = AlertPolicy::default();
        let plain = snap(97.0, 200, Some(2.0), Some(5.0));
        let paging = snap(97.0, 200, Some(900.0), Some(100.0));
        let ok = snap(50.0, 200, Some(1.0), Some(5.0));
        assert_eq!(p.evaluate(0, conds(&plain)).len(), 1);
        assert!(p.evaluate(MIN, conds(&ok)).is_empty());
        assert!(p.evaluate(4 * MIN, conds(&ok)).is_empty());
        assert_eq!(p.evaluate(5 * MIN, conds(&paging)).len(), 1);
    }

    #[test]
    fn drives_are_tracked_independently() {
        let mut p = AlertPolicy::default();
        let mk = |mount: &str| Alert {
            key: format!("disk:{mount}"),
            kind: AlertKind::Disk,
            level: 2,
            title: String::new(),
            body: String::new(),
            destination: Destination::Diagnosis,
            user_initiated: false,
        };
        assert_eq!(p.evaluate(0, vec![mk("C:\\")]).len(), 1);
        assert_eq!(
            p.evaluate(1_000, vec![mk("C:\\"), mk("S:\\")]).len(),
            1,
            "only the new drive"
        );
    }

    #[test]
    fn user_results_are_skipped_while_the_window_is_visible() {
        let mut p = AlertPolicy::default();
        let a = scan_alert(12, 8 * GIB, true, 1);
        assert!(!p.admit_result(&a, 0, true));
        assert!(p.admit_result(&a, 0, false));
        assert!(
            !p.admit_result(&a, 60_000, false),
            "the same scan is never announced twice"
        );
    }

    #[test]
    fn automated_actions_always_notify_and_results_are_rate_limited() {
        let mut p = AlertPolicy::default();
        let auto = action_alert(1, "Clean old temporary files", true, "Freed 1.2 GB.", true);
        assert!(
            p.admit_result(&auto, 0, true),
            "nobody is watching an automated run"
        );
        let next = action_alert(2, "Stop Gradle daemons", true, "Done.", false);
        assert!(
            !p.admit_result(&next, 1_000, false),
            "within 5 s of the previous action toast"
        );
        assert!(p.admit_result(&next, 10_000, false));
    }

    #[test]
    fn incident_toast_carries_the_hedged_diagnosis_and_goes_to_history() {
        let a = incident_alert(
            5,
            48,
            "Memory exhaustion caused heavy paging, which saturated the disk and likely caused the system responsiveness problem.",
        );
        assert!(a.title.contains("48 s"));
        assert!(a.body.contains("likely caused"));
        assert_eq!(a.destination.route(), "/history");
        assert_eq!(a.destination.button(), "View incident");
    }

    #[test]
    fn long_bodies_are_clipped() {
        let a = incident_alert(1, 1, &"x".repeat(1000));
        assert!(a.body.chars().count() <= BODY_MAX);
        assert!(a.body.ends_with('…'));
    }

    #[test]
    fn scan_toast_says_when_the_result_is_partial() {
        assert!(scan_alert(3, GIB, false, 1).body.contains("partial"));
        assert!(!scan_alert(3, GIB, true, 1).body.contains("partial"));
    }

    #[test]
    fn destinations_are_all_known_app_routes() {
        for d in [
            Destination::Diagnosis,
            Destination::Storage,
            Destination::History,
            Destination::Actions,
        ] {
            assert!(
                crate::windows::window::is_allowed_route(d.route()),
                "{}",
                d.route()
            );
        }
    }
}
