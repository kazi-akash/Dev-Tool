//! Background sampling thread. Runs independent of the UI so monitoring continues in the tray.

use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

use super::config::MonitorInterval;
use super::metrics::{DiskMetrics, SystemSnapshot, clamp_cpu, disk_metrics, memory_metrics};

pub const EVENT_SYSTEM_METRICS: &str = "system-metrics";

/// Disk lists change rarely and refreshing them touches the filesystem; sample every N ticks.
const DISK_REFRESH_EVERY_TICKS: u64 = 5;

/// Latest snapshot, shared between the sampler thread and IPC commands.
#[derive(Default)]
pub struct MonitorState {
    pub interval: MonitorInterval,
    latest: RwLock<Option<SystemSnapshot>>,
}

impl MonitorState {
    pub fn latest(&self) -> Option<SystemSnapshot> {
        // A poisoned lock only means a sampler panicked mid-write; the old value is still usable.
        self.latest
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn store(&self, snapshot: SystemSnapshot) {
        *self.latest.write().unwrap_or_else(|e| e.into_inner()) = Some(snapshot);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn collect_disks(disks: &Disks) -> Vec<DiskMetrics> {
    disks
        .iter()
        .filter(|d| d.total_space() > 0 && !d.is_removable())
        .map(|d| {
            disk_metrics(
                d.name().to_string_lossy().into_owned(),
                d.mount_point().to_string_lossy().into_owned(),
                d.total_space(),
                d.available_space(),
            )
        })
        .collect()
}

/// Starts the sampler. Returns immediately; the thread lives for the whole app lifetime.
pub fn start(app: AppHandle, state: Arc<MonitorState>) {
    let spawned = thread::Builder::new()
        .name("devpilot-monitor".into())
        .spawn(move || run(app, state));
    if let Err(e) = spawned {
        // Graceful degradation: the app keeps running without live metrics.
        eprintln!("devpilot: failed to start monitor thread: {e}");
    }
}

fn run(app: AppHandle, state: Arc<MonitorState>) {
    let mut sys = match System::new() {
        Ok(sys) => sys,
        Err(e) => {
            // Graceful degradation: no live metrics, but the rest of the app keeps working.
            eprintln!("devpilot: system monitor unavailable: {e}");
            return;
        }
    };
    // Disk enumeration may fail independently (permissions, odd volumes); retried on the disk cadence.
    let mut disks = Disks::new_with_refreshed_list().ok();
    let mut disk_cache = disks.as_ref().map(collect_disks).unwrap_or_default();
    let mut tick: u64 = 0;

    // CPU usage needs two samples; prime it so the first real reading is meaningful.
    sys.refresh_cpu_usage();
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        // Count only: no per-process details requested, keeps this cheap.
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );

        if tick > 0 && tick.is_multiple_of(DISK_REFRESH_EVERY_TICKS) {
            match disks.as_mut() {
                Some(d) => d.refresh(true),
                None => disks = Disks::new_with_refreshed_list().ok(),
            }
            if let Some(d) = disks.as_ref() {
                disk_cache = collect_disks(d);
            }
        }

        let snapshot = SystemSnapshot {
            timestamp_ms: now_ms(),
            memory: memory_metrics(sys.total_memory(), sys.available_memory()),
            cpu_percent: clamp_cpu(sys.global_cpu_usage()),
            disks: disk_cache.clone(),
            uptime_secs: System::uptime().unwrap_or(0),
            process_count: sys.processes().len(),
        };
        state.store(snapshot.clone());

        // Skip the IPC push while the window is hidden in the tray; the snapshot is still kept fresh.
        let visible = app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        if visible {
            let _ = app.emit(EVENT_SYSTEM_METRICS, &snapshot);
        }

        tick = tick.wrapping_add(1);
        thread::sleep(state.interval.get());
    }
}
