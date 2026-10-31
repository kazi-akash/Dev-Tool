//! Opt-in safe automation: when enabled and a drive is running low, clean old temporary files.
//! It can only invoke actions the engine marks automation-eligible and LOW risk; there is no path
//! from here to deleting arbitrary files, closing processes, or touching the registry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::engine::ActionEngine;
use super::model::{Ctx, Facts};
use super::real::{RealKiller, RealRunner};
use crate::monitoring::disks::DiskState;
use crate::monitoring::service::MonitorState;

const CHECK_EVERY: Duration = Duration::from_secs(60);
const MIN_GAP: Duration = Duration::from_secs(3600);
const LOW_FREE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// The only action automation may run.
pub const AUTOMATED_ACTION: &str = "cleanup.temp";

#[derive(Default)]
pub struct AutomationState {
    enabled: AtomicBool,
    last_run: Mutex<Option<Instant>>,
}

impl AutomationState {
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
}

/// Pure decision: should the automation run now?
pub fn should_run(
    enabled: bool,
    worst: Option<DiskState>,
    min_free: Option<u64>,
    since_last: Option<Duration>,
) -> bool {
    if !enabled {
        return false;
    }
    if since_last.is_some_and(|d| d < MIN_GAP) {
        return false;
    }
    worst.is_some_and(|s| s >= DiskState::High) || min_free.is_some_and(|f| f < LOW_FREE_BYTES)
}

#[derive(Default)]
pub struct ActionsState {
    pub engine: ActionEngine,
    pub automation: AutomationState,
}

pub fn start(actions: Arc<ActionsState>, monitor: Arc<MonitorState>) {
    let spawned = thread::Builder::new()
        .name("devpilot-automation".into())
        .spawn(move || {
            loop {
                thread::sleep(CHECK_EVERY);
                if !actions.automation.enabled() {
                    continue;
                }
                let Some(snap) = monitor.latest() else {
                    continue;
                };
                let worst = snap.disk_health.iter().map(|d| d.state).max();
                let min_free = snap.disks.iter().map(|d| d.free_bytes).min();
                let since = actions
                    .automation
                    .last_run
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|t| t.elapsed());
                if !should_run(true, worst, min_free, since) {
                    continue;
                }
                let facts = Facts {
                    temp_dir: std::env::var_os("TEMP").map(Into::into),
                    user_profile: std::env::var_os("USERPROFILE").map(Into::into),
                    ..Default::default()
                };
                let ctx = Ctx {
                    facts: &facts,
                    runner: &RealRunner,
                    killer: &RealKiller,
                };
                // Success or failure, wait before trying again; the attempt is in the action history either way.
                *actions
                    .automation
                    .last_run
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                let _ = actions
                    .engine
                    .execute_automated(AUTOMATED_ACTION, &json!({}), &ctx);
            }
        });
    if let Err(e) = spawned {
        eprintln!("devpilot: failed to start automation thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn disabled_never_runs() {
        assert!(!should_run(
            false,
            Some(DiskState::Critical),
            Some(GB),
            None
        ));
    }

    #[test]
    fn runs_when_a_drive_is_high_or_critical() {
        assert!(should_run(true, Some(DiskState::High), Some(50 * GB), None));
        assert!(should_run(
            true,
            Some(DiskState::Critical),
            Some(50 * GB),
            None
        ));
        assert!(!should_run(
            true,
            Some(DiskState::Warning),
            Some(50 * GB),
            None
        ));
        assert!(!should_run(
            true,
            Some(DiskState::Normal),
            Some(50 * GB),
            None
        ));
    }

    #[test]
    fn runs_when_absolute_free_space_is_low() {
        assert!(should_run(
            true,
            Some(DiskState::Normal),
            Some(9 * GB),
            None
        ));
        assert!(!should_run(
            true,
            Some(DiskState::Normal),
            Some(11 * GB),
            None
        ));
    }

    #[test]
    fn no_data_means_no_run() {
        assert!(!should_run(true, None, None, None));
    }

    #[test]
    fn respects_the_one_hour_gap() {
        assert!(!should_run(
            true,
            Some(DiskState::Critical),
            None,
            Some(Duration::from_secs(600))
        ));
        assert!(should_run(
            true,
            Some(DiskState::Critical),
            None,
            Some(Duration::from_secs(3601))
        ));
    }

    #[test]
    fn only_temp_cleanup_is_automated_and_it_is_eligible() {
        let engine = ActionEngine::default();
        let spec = engine
            .list()
            .into_iter()
            .find(|s| s.id == AUTOMATED_ACTION)
            .unwrap();
        assert!(spec.automation_eligible);
        assert_eq!(
            engine
                .list()
                .iter()
                .filter(|s| s.automation_eligible)
                .count(),
            1
        );
    }

    #[test]
    fn off_by_default_and_toggles() {
        let s = AutomationState::default();
        assert!(!s.enabled());
        s.set_enabled(true);
        assert!(s.enabled());
    }
}
