//! Background scan lifecycle: start, observe, cancel. One scan at a time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;

use super::governor::GovernorHandle;
use super::scanner::{
    DEEP_TIME_BUDGET, ScanEnv, ScanOptions, ScanProgress, ScanResult, run_deep_appdata_scan,
    run_scan,
};
type DoneHook = Box<dyn Fn(&ScanResult) + Send + Sync>;

#[derive(Default)]
pub struct StorageState {
    running: AtomicBool,
    cancel: AtomicBool,
    progress: Mutex<ScanProgress>,
    result: Mutex<Option<ScanResult>>,
    /// Result of the optional deep AppData scan. Kept apart so it never replaces the standard
    /// result, which is the whitelist the cleanup action checks against.
    deep_result: Mutex<Option<ScanResult>>,
    /// The scan now running is a deep scan.
    deep_running: AtomicBool,
    error: Mutex<Option<String>>,
    /// Called once with each finished (or stopped) scan; used for the completion notification.
    on_done: std::sync::OnceLock<DoneHook>,
    /// Decides when scans run, slow down or pause. Set once at startup.
    governor: std::sync::OnceLock<GovernorHandle>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    pub running: bool,
    pub progress: ScanProgress,
    pub result: Option<ScanResult>,
    /// Latest deep AppData scan, if the user ever ran one.
    pub deep_result: Option<ScanResult>,
    /// What is running now (or last ran): a deep AppData scan.
    pub deep: bool,
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl StorageState {
    /// Start a scan unless one is already running. Returns false if one was already running.
    pub fn start(self: &Arc<Self>, env: ScanEnv, opts: ScanOptions) -> bool {
        self.start_kind(env, opts, false)
    }

    /// Start the optional deep AppData scan. Only ever called for an explicit user request.
    pub fn start_deep(self: &Arc<Self>, env: ScanEnv, mut opts: ScanOptions) -> bool {
        opts.time_budget = DEEP_TIME_BUDGET;
        self.start_kind(env, opts, true)
    }

    fn start_kind(self: &Arc<Self>, env: ScanEnv, mut opts: ScanOptions, deep: bool) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        self.deep_running.store(deep, Ordering::SeqCst);
        *lock(&self.error) = None;
        *lock(&self.progress) = ScanProgress {
            phase: "Starting".into(),
            ..Default::default()
        };

        if opts.governor.is_none() {
            opts.governor = self.governor.get().cloned();
        }
        let state = self.clone();
        let spawned = thread::Builder::new()
            .name("devpilot-storage-scan".into())
            .spawn(move || {
                // Background priority: low CPU, I/O and memory priority for this thread only.
                crate::windows::priority::enter_background_mode();
                // A panic inside the scan must not leave the UI waiting forever.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if deep {
                        run_deep_appdata_scan(&env, &opts, &state.cancel, &state.progress)
                    } else {
                        run_scan(&env, &opts, &state.cancel, &state.progress)
                    }
                }));
                match outcome {
                    Ok(result) => {
                        if let Some(hook) = state.on_done.get() {
                            hook(&result);
                        }
                        if deep {
                            *lock(&state.deep_result) = Some(result);
                        } else {
                            *lock(&state.result) = Some(result);
                        }
                    }
                    Err(_) => {
                        *lock(&state.error) = Some("The storage scan failed unexpectedly.".into())
                    }
                }
                state.running.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            *lock(&self.error) = Some("Could not start the scan thread.".into());
            self.running.store(false, Ordering::SeqCst);
        }
        true
    }

    /// Register the governor. Only the first call has an effect.
    pub fn set_governor(&self, g: GovernorHandle) {
        let _ = self.governor.set(g);
    }

    /// Register the completion hook. Only the first call has an effect.
    pub fn set_on_done(&self, f: impl Fn(&ScanResult) + Send + Sync + 'static) {
        let _ = self.on_done.set(Box::new(f));
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn status(&self) -> ScanStatus {
        ScanStatus {
            running: self.running.load(Ordering::SeqCst),
            progress: lock(&self.progress).clone(),
            result: lock(&self.result).clone(),
            deep_result: lock(&self.deep_result).clone(),
            deep: self.deep_running.load(Ordering::SeqCst),
            error: lock(&self.error).clone(),
        }
    }

    /// Latest completed (or partial) result, for the cleanup candidate checks.
    pub fn latest_result(&self) -> Option<ScanResult> {
        lock(&self.result).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_done(s: &StorageState) {
        let end = Instant::now() + Duration::from_secs(10);
        while s.status().running && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn runs_in_background_and_stores_result() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        std::fs::create_dir_all(root.join("a\\node_modules")).unwrap();
        std::fs::write(root.join("a\\package.json"), b"{}").unwrap();
        std::fs::write(root.join("a\\node_modules\\x.js"), vec![0u8; 100]).unwrap();

        let s = Arc::new(StorageState::default());
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        assert!(s.start(env, ScanOptions::default()));
        wait_done(&s);
        let st = s.status();
        assert!(!st.running && st.error.is_none());
        assert_eq!(st.result.unwrap().entries.len(), 1);
    }

    #[test]
    fn second_start_while_running_is_rejected() {
        let s = Arc::new(StorageState::default());
        s.running.store(true, Ordering::SeqCst);
        assert!(!s.start(ScanEnv::default(), ScanOptions::default()));
    }

    #[test]
    fn can_run_again_after_finishing() {
        let s = Arc::new(StorageState::default());
        assert!(s.start(ScanEnv::default(), ScanOptions::default()));
        wait_done(&s);
        assert!(s.start(ScanEnv::default(), ScanOptions::default()));
        wait_done(&s);
    }

    #[test]
    fn status_before_any_scan_is_idle() {
        let st = StorageState::default().status();
        assert!(!st.running && st.result.is_none() && st.error.is_none());
    }
}

#[cfg(test)]
mod hook_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    #[test]
    fn completion_hook_runs_once_per_scan_with_the_result() {
        let s = Arc::new(StorageState::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        s.set_on_done(move |r| {
            assert!(r.complete);
            seen.fetch_add(1, Ordering::SeqCst);
        });
        // A second registration is ignored.
        s.set_on_done(|_| panic!("only the first hook counts"));
        assert!(s.start(ScanEnv::default(), ScanOptions::default()));
        let end = Instant::now() + Duration::from_secs(10);
        while s.status().running && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(s.latest_result().is_some());
    }
}

#[cfg(test)]
mod deep_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_done(s: &StorageState) {
        let end = Instant::now() + Duration::from_secs(20);
        while s.status().running && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_deep_scan_never_replaces_the_standard_result_used_for_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        std::fs::create_dir_all(root.join("a/node_modules")).unwrap();
        std::fs::write(root.join("a/package.json"), b"{}").unwrap();
        std::fs::write(root.join("a/node_modules/x.js"), vec![0u8; 100]).unwrap();
        let ad = tmp.path().join("AppData/Local/Slack");
        std::fs::create_dir_all(&ad).unwrap();
        std::fs::File::create(ad.join("c.bin"))
            .unwrap()
            .set_len(120 * 1024 * 1024)
            .unwrap();

        let s = Arc::new(StorageState::default());
        let env = ScanEnv {
            dev_roots: vec![root],
            local_app_data: Some(tmp.path().join("AppData/Local")),
            ..Default::default()
        };
        assert!(s.start(env.clone(), ScanOptions::default()));
        wait_done(&s);
        assert_eq!(s.latest_result().unwrap().entries.len(), 1);

        assert!(s.start_deep(env, ScanOptions::default()));
        wait_done(&s);
        let st = s.status();
        let deep = st.deep_result.expect("deep result");
        assert_eq!(deep.entries.len(), 1);
        assert_eq!(deep.entries[0].name, "Slack");
        // The cleanup whitelist is untouched.
        let standard = st.result.unwrap();
        assert_eq!(standard.entries.len(), 1);
        assert!(standard.entries[0].path.contains("node_modules"));
        assert_eq!(
            s.latest_result().unwrap().entries[0].path,
            standard.entries[0].path
        );
    }

    #[test]
    fn deep_and_standard_scans_never_run_together() {
        let s = Arc::new(StorageState::default());
        s.running.store(true, Ordering::SeqCst);
        assert!(!s.start_deep(ScanEnv::default(), ScanOptions::default()));
        assert!(!s.start(ScanEnv::default(), ScanOptions::default()));
    }
}
