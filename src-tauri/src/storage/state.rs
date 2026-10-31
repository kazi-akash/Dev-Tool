//! Background scan lifecycle: start, observe, cancel. One scan at a time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;

use super::scanner::{ScanEnv, ScanOptions, ScanProgress, ScanResult, run_scan};

#[derive(Default)]
pub struct StorageState {
    running: AtomicBool,
    cancel: AtomicBool,
    progress: Mutex<ScanProgress>,
    result: Mutex<Option<ScanResult>>,
    error: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    pub running: bool,
    pub progress: ScanProgress,
    pub result: Option<ScanResult>,
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl StorageState {
    /// Start a scan unless one is already running. Returns false if one was already running.
    pub fn start(self: &Arc<Self>, env: ScanEnv, opts: ScanOptions) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        *lock(&self.error) = None;
        *lock(&self.progress) = ScanProgress {
            phase: "Starting".into(),
            ..Default::default()
        };

        let state = self.clone();
        let spawned = thread::Builder::new()
            .name("devpilot-storage-scan".into())
            .spawn(move || {
                // A panic inside the scan must not leave the UI waiting forever.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_scan(&env, &opts, &state.cancel, &state.progress)
                }));
                match outcome {
                    Ok(result) => *lock(&state.result) = Some(result),
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

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn status(&self) -> ScanStatus {
        ScanStatus {
            running: self.running.load(Ordering::SeqCst),
            progress: lock(&self.progress).clone(),
            result: lock(&self.result).clone(),
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
