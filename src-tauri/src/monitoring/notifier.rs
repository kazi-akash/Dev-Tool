//! Glue between the alert policy and the OS toast. The policy decides; a `Toaster` displays.
//! The master switch lives here so the UI setting is enforced in Rust, not just hidden in the UI.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::alerts::{Alert, AlertKind, AlertPolicy, Destination};
use super::metrics::SystemSnapshot;

/// Shows a toast. Real on Windows, scripted in tests.
pub trait Toaster: Send + Sync {
    fn show(&self, alert: &Alert) -> Result<(), String>;
    /// Whether the main window is on screen (user-initiated results are then skipped).
    fn window_visible(&self) -> bool;
}

pub struct Notifier {
    policy: Mutex<AlertPolicy>,
    enabled: AtomicBool,
    toaster: Box<dyn Toaster>,
}

impl Notifier {
    pub fn new(toaster: Box<dyn Toaster>) -> Self {
        Self {
            policy: Mutex::new(AlertPolicy::default()),
            // On by default: only critical conditions and results the user asked for notify.
            enabled: AtomicBool::new(true),
            toaster,
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn policy(&self) -> std::sync::MutexGuard<'_, AlertPolicy> {
        self.policy.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn deliver(&self, alert: &Alert) {
        if let Err(e) = self.toaster.show(alert) {
            // A toast that cannot be shown must never disturb monitoring.
            eprintln!("devpilot: notification not shown: {e}");
        }
    }

    /// Called every sample. Cheap: builds a few small structs and compares them.
    pub fn evaluate_snapshot(
        &self,
        snapshot: &SystemSnapshot,
        top: Option<(&str, u64)>,
        now_ms: u64,
    ) {
        let conditions = AlertPolicy::conditions(snapshot, top);
        // Keep evaluating when disabled so a later re-enable does not replay old conditions.
        let admitted = self.policy().evaluate(now_ms, conditions);
        if self.enabled() {
            for a in &admitted {
                self.deliver(a);
            }
        }
    }

    /// One-off results: scan finished, action finished, incident recorded.
    pub fn result(&self, alert: &Alert, now_ms: u64) {
        if !self.enabled() {
            return;
        }
        let visible = self.toaster.window_visible();
        if self.policy().admit_result(alert, now_ms, visible) {
            self.deliver(alert);
        }
    }

    /// A harmless toast the user can request to see that notifications work on their system.
    pub fn test(&self) -> Result<(), String> {
        self.toaster.show(&Alert {
            key: "test".into(),
            kind: AlertKind::ActionComplete,
            level: 0,
            title: "DevPilot notifications work".into(),
            body: "You will be told when RAM or a drive becomes critical, when memory escalates, and when a scan, action or incident finishes.".into(),
            destination: Destination::Diagnosis,
            user_initiated: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Fake {
        shown: Mutex<Vec<String>>,
        visible: AtomicBool,
        fail: AtomicBool,
    }
    struct FakeToaster(Arc<Fake>);
    impl Toaster for FakeToaster {
        fn show(&self, a: &Alert) -> Result<(), String> {
            if self.0.fail.load(Ordering::Relaxed) {
                return Err("no display".into());
            }
            self.0.shown.lock().unwrap().push(a.title.clone());
            Ok(())
        }
        fn window_visible(&self) -> bool {
            self.0.visible.load(Ordering::Relaxed)
        }
    }

    fn rig() -> (Notifier, Arc<Fake>) {
        let f = Arc::new(Fake::default());
        (Notifier::new(Box::new(FakeToaster(f.clone()))), f)
    }

    fn shown(f: &Fake) -> Vec<String> {
        f.shown.lock().unwrap().clone()
    }

    #[test]
    fn disabled_notifier_shows_nothing_but_forgets_nothing() {
        let (n, f) = rig();
        n.set_enabled(false);
        n.result(&crate::monitoring::alerts::scan_alert(1, 1, true, 1), 0);
        assert!(shown(&f).is_empty());
        n.set_enabled(true);
        n.result(
            &crate::monitoring::alerts::scan_alert(1, 1, true, 2),
            10_000,
        );
        assert_eq!(shown(&f).len(), 1);
    }

    #[test]
    fn window_visibility_suppresses_user_results_only() {
        let (n, f) = rig();
        f.visible.store(true, Ordering::Relaxed);
        n.result(&crate::monitoring::alerts::scan_alert(1, 1, true, 1), 0);
        assert!(shown(&f).is_empty(), "the user is looking at the scan page");
        n.result(
            &crate::monitoring::alerts::incident_alert(1, 30, "x"),
            10_000,
        );
        assert_eq!(
            shown(&f).len(),
            1,
            "an incident is not something the user just did"
        );
    }

    #[test]
    fn a_toast_that_fails_is_swallowed() {
        let (n, f) = rig();
        f.fail.store(true, Ordering::Relaxed);
        n.result(&crate::monitoring::alerts::incident_alert(1, 30, "x"), 0);
        assert!(n.test().is_err(), "the test button reports the failure");
    }

    #[test]
    fn test_toast_is_always_shown() {
        let (n, f) = rig();
        f.visible.store(true, Ordering::Relaxed);
        n.test().unwrap();
        assert_eq!(shown(&f), vec!["DevPilot notifications work"]);
    }
}
