//! Preferences that the Rust side owns (sampling interval, thresholds, safe automation,
//! notifications), persisted next to the history database.
//!
//! The UI keeps its own copy in localStorage and pushes it to Rust whenever the window opens. That
//! is not enough once DevPilot can start minimized with Windows: no window means no push, so the
//! background would run on defaults (for example toasts the user turned off). Rust therefore loads
//! these values at startup itself. Every value is validated again when applied; a corrupt or
//! hand-edited file can never put the monitor into an invalid state.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::actions::automation::AutomationState;
use crate::monitoring::disks::DiskThresholds;
use crate::monitoring::pressure::PressureThresholds;
use crate::monitoring::service::{MonitorState, lock};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub monitor_interval_secs: Option<u64>,
    /// `[moderate, high, critical]`
    pub pressure: Option<[f32; 3]>,
    /// `[warning_free, high_free, critical_free]`
    pub disk: Option<[f32; 3]>,
    pub safe_automation: Option<bool>,
    pub notifications: Option<bool>,
}

impl Prefs {
    /// Missing, unreadable or corrupt files give defaults; preferences are never worth failing for.
    pub fn load(path: &Path) -> Prefs {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write through a temporary file so a crash cannot leave half a file behind.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(&tmp, path)
    }
}

/// Live preferences plus the file they are saved to (`None` when there is no data folder).
pub struct PrefsStore {
    path: Option<PathBuf>,
    current: Mutex<Prefs>,
}

impl PrefsStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        let current = path.as_deref().map(Prefs::load).unwrap_or_default();
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn snapshot(&self) -> Prefs {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Change one value and save, but only when it actually changed (the UI re-sends every
    /// setting each time the window opens).
    pub fn update(&self, change: impl FnOnce(&mut Prefs)) {
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let before = cur.clone();
        change(&mut cur);
        if *cur != before
            && let Some(p) = &self.path
            && let Err(e) = cur.save(p)
        {
            eprintln!("devpilot: could not save preferences: {e}");
        }
    }
}

/// Apply stored preferences at startup. Invalid values are skipped, not clamped.
pub fn apply(prefs: &Prefs, monitor: &MonitorState, automation: &AutomationState) {
    if let Some(s) = prefs.monitor_interval_secs {
        let _ = monitor.interval.set_secs(s);
    }
    if let Some([m, h, c]) = prefs.pressure
        && let Ok(t) = PressureThresholds::validated(m, h, c)
    {
        lock(&monitor.pressure).set_thresholds(t);
    }
    if let Some([w, h, c]) = prefs.disk
        && let Ok(t) = DiskThresholds::validated(w, h, c)
    {
        lock(&monitor.disks).set_thresholds(t);
    }
    if let Some(on) = prefs.safe_automation {
        automation.set_enabled(on);
    }
    if let Some(on) = prefs.notifications
        && let Some(n) = monitor.notifier.get()
    {
        n.set_enabled(on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn file() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("data").join("prefs.json");
        (d, p)
    }

    #[test]
    fn round_trips_through_the_file() {
        let (_d, p) = file();
        let s = PrefsStore::new(Some(p.clone()));
        s.update(|x| x.notifications = Some(false));
        s.update(|x| x.pressure = Some([60.0, 80.0, 90.0]));
        let again = PrefsStore::new(Some(p));
        assert_eq!(again.snapshot().notifications, Some(false));
        assert_eq!(again.snapshot().pressure, Some([60.0, 80.0, 90.0]));
    }

    #[test]
    fn missing_and_corrupt_files_give_defaults() {
        let (_d, p) = file();
        assert_eq!(Prefs::load(&p), Prefs::default());
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, "{ this is not json").unwrap();
        assert_eq!(Prefs::load(&p), Prefs::default());
        fs::write(&p, r#"{"notifications": "yes please"}"#).unwrap();
        assert_eq!(
            Prefs::load(&p),
            Prefs::default(),
            "wrong types are not half-applied"
        );
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let (_d, p) = file();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, r#"{"notifications": false, "someFutureThing": 1}"#).unwrap();
        let l = Prefs::load(&p);
        assert_eq!(l.notifications, Some(false));
        assert_eq!(l.monitor_interval_secs, None);
    }

    #[test]
    fn unchanged_values_do_not_rewrite_the_file() {
        let (_d, p) = file();
        let s = PrefsStore::new(Some(p.clone()));
        s.update(|x| x.safe_automation = Some(true));
        let first = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        s.update(|x| x.safe_automation = Some(true));
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), first);
        assert!(
            !p.with_extension("json.tmp").exists(),
            "no temporary file is left behind"
        );
    }

    #[test]
    fn no_data_folder_means_in_memory_only() {
        let s = PrefsStore::new(None);
        s.update(|x| x.notifications = Some(false));
        assert_eq!(s.snapshot().notifications, Some(false));
    }

    #[test]
    fn apply_sets_valid_values_and_skips_invalid_ones() {
        let monitor = MonitorState::default();
        let automation = AutomationState::default();
        apply(
            &Prefs {
                monitor_interval_secs: Some(5),
                pressure: Some([60.0, 80.0, 90.0]),
                disk: Some([25.0, 12.0, 6.0]),
                safe_automation: Some(true),
                notifications: Some(false),
            },
            &monitor,
            &automation,
        );
        assert_eq!(monitor.interval.get(), Duration::from_secs(5));
        assert_eq!(lock(&monitor.pressure).thresholds().high, 80.0);
        assert_eq!(lock(&monitor.disks).thresholds().critical_free, 6.0);
        assert!(automation.enabled());

        // Hand-edited garbage changes nothing.
        let monitor = MonitorState::default();
        apply(
            &Prefs {
                monitor_interval_secs: Some(0),
                pressure: Some([95.0, 80.0, 70.0]),
                disk: Some([1.0, 2.0, 3.0]),
                ..Default::default()
            },
            &monitor,
            &AutomationState::default(),
        );
        assert_eq!(monitor.interval.get(), Duration::from_secs(2));
        assert_eq!(
            lock(&monitor.pressure).thresholds(),
            PressureThresholds::default()
        );
        assert_eq!(lock(&monitor.disks).thresholds(), DiskThresholds::default());
    }
}
