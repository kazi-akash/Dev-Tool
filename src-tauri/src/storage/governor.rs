//! Keeps the storage scan from becoming a source of pressure itself.
//!
//! The scanner asks a *governor* every few hundred entries whether it may continue. The decision is
//! a pure function of conditions DevPilot already measures, so it costs nothing extra:
//!
//! * **Pause** while memory pressure is CRITICAL, a performance incident is being recorded, or an
//!   Android/Gradle/Flutter build is running. The scan resumes by itself when that ends, and the time
//!   spent paused does not count against the scan's time budget.
//! * **Throttle** (longer sleeps) while pressure is HIGH or another build/startup is running.
//! * Otherwise **Run**, still with a small pause every few hundred entries so the scan never
//!   saturates the disk.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::monitoring::pressure::PressureState;

/// Entries visited between two governor checks.
pub const CHECK_EVERY: usize = 256;
/// Pause between checks while running normally.
pub const RUN_SLEEP: Duration = Duration::from_millis(1);
/// Pause between checks while throttled.
pub const THROTTLE_SLEEP: Duration = Duration::from_millis(15);
/// How often a paused scan looks again.
pub const PAUSE_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Run,
    Throttle(String),
    Pause(String),
}

/// What the decision depends on.
#[derive(Debug, Clone, Default)]
pub struct Conditions {
    pub pressure: Option<PressureState>,
    /// Labels of the running developer sessions (see `developers::sessions`).
    pub active_sessions: Vec<String>,
    pub incident_active: bool,
}

/// Session labels that load the machine hard enough to pause a scan.
fn is_intensive(label: &str) -> bool {
    matches!(label, "Android build" | "Gradle build" | "Flutter build")
}

pub fn decide(c: &Conditions) -> Gate {
    if c.pressure == Some(PressureState::Critical) {
        return Gate::Pause("memory pressure is critical".into());
    }
    if c.incident_active {
        return Gate::Pause("a performance incident is being recorded".into());
    }
    if let Some(l) = c.active_sessions.iter().find(|l| is_intensive(l)) {
        return Gate::Pause(format!("a {} is running", l.to_lowercase()));
    }
    if c.pressure == Some(PressureState::High) {
        return Gate::Throttle("memory pressure is high".into());
    }
    if let Some(l) = c.active_sessions.first() {
        return Gate::Throttle(format!("a {} is running", l.to_lowercase()));
    }
    Gate::Run
}

/// A shareable "may I continue?" function. Cheap to clone; manual `Debug` keeps `ScanOptions` debuggable.
#[derive(Clone)]
pub struct GovernorHandle(pub Arc<dyn Fn() -> Gate + Send + Sync>);

impl GovernorHandle {
    pub fn new(f: impl Fn() -> Gate + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    pub fn check(&self) -> Gate {
        (self.0)()
    }
}

impl fmt::Debug for GovernorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GovernorHandle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(p: Option<PressureState>, s: &[&str], incident: bool) -> Conditions {
        Conditions {
            pressure: p,
            active_sessions: s.iter().map(|x| x.to_string()).collect(),
            incident_active: incident,
        }
    }

    #[test]
    fn calm_machine_runs() {
        assert_eq!(
            decide(&c(Some(PressureState::Normal), &[], false)),
            Gate::Run
        );
        assert_eq!(
            decide(&c(Some(PressureState::Moderate), &[], false)),
            Gate::Run
        );
        assert_eq!(decide(&c(None, &[], false)), Gate::Run);
    }

    #[test]
    fn critical_pressure_and_incidents_pause() {
        assert!(
            matches!(decide(&c(Some(PressureState::Critical), &[], false)), Gate::Pause(r) if r.contains("critical"))
        );
        assert!(
            matches!(decide(&c(Some(PressureState::Normal), &[], true)), Gate::Pause(r) if r.contains("incident"))
        );
    }

    #[test]
    fn android_gradle_and_flutter_builds_pause() {
        for s in ["Android build", "Gradle build", "Flutter build"] {
            assert!(
                matches!(decide(&c(Some(PressureState::Normal), &[s], false)), Gate::Pause(r) if r.contains("running")),
                "{s}"
            );
        }
    }

    #[test]
    fn lighter_activity_only_throttles() {
        assert!(matches!(
            decide(&c(Some(PressureState::High), &[], false)),
            Gate::Throttle(_)
        ));
        for s in [
            "Next.js build",
            "npm/pnpm build",
            "Docker compose startup",
            "Android emulator startup",
        ] {
            assert!(
                matches!(
                    decide(&c(Some(PressureState::Normal), &[s], false)),
                    Gate::Throttle(_)
                ),
                "{s}"
            );
        }
    }

    #[test]
    fn the_worst_condition_wins() {
        assert!(matches!(
            decide(&c(Some(PressureState::High), &["Flutter build"], false)),
            Gate::Pause(_)
        ));
        assert!(
            matches!(decide(&c(Some(PressureState::Critical), &["Next.js build"], true)), Gate::Pause(r) if r.contains("critical"))
        );
    }

    #[test]
    fn handle_calls_through_and_is_debuggable() {
        let h = GovernorHandle::new(|| Gate::Throttle("x".into()));
        assert_eq!(h.check(), Gate::Throttle("x".into()));
        assert_eq!(format!("{h:?}"), "GovernorHandle");
    }
}
