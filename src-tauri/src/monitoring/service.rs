//! Background sampling thread. Runs independent of the UI so monitoring continues in the tray.

use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager};

use super::config::MonitorInterval;
use super::disks::DiskEngine;
use super::growth::GrowthTracker;
use super::metrics::{DiskMetrics, SystemSnapshot, clamp_cpu, disk_metrics, memory_metrics};
use super::incident::{IncidentRecorder, IncidentSample, NamedBytes};
use super::pressure::{PressureEngine, PressureInputs, PressureState, Trend};
use super::telemetry::{TelemetryEngine, TelemetryInputs};
use crate::processes::collector::ProcessCollector;
use crate::processes::store::ProcessStore;
use crate::windows::memory::read_commit_info;

/// Engines hold plain data; a panic in another thread must not disable monitoring.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub const EVENT_SYSTEM_METRICS: &str = "system-metrics";

/// Disk lists change rarely and refreshing them touches the filesystem; sample every N ticks.
const DISK_REFRESH_EVERY_TICKS: u64 = 5;

/// Latest snapshot, shared between the sampler thread and IPC commands.
#[derive(Default)]
pub struct MonitorState {
    pub interval: MonitorInterval,
    pub processes: ProcessStore,
    pub pressure: Mutex<PressureEngine>,
    pub disks: Mutex<DiskEngine>,
    pub growth: Mutex<GrowthTracker>,
    pub telemetry: Mutex<TelemetryEngine>,
    pub incident: Mutex<IncidentRecorder>,
    pub sessions: Mutex<crate::developers::sessions::SessionTracker>,
    /// Filled once the history database is open; the sampler records into it when present.
    pub history: crate::commands::history::HistorySlot,
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
    let mut collector = ProcessCollector::default();
    // PDH handles stay on this thread for life.
    let mut perf = crate::windows::perf::PerfCounters::new();
    let started = Instant::now();

    // CPU usage needs two samples; prime it so the first real reading is meaningful.
    sys.refresh_cpu_usage();
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        // Exe path and command line are fetched once per process (OnlyIfNotSet); only CPU and
        // memory are re-read every tick.
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        let process_list = collector.collect(&sys);
        state.processes.replace(process_list.clone());

        if tick > 0 && tick.is_multiple_of(DISK_REFRESH_EVERY_TICKS) {
            match disks.as_mut() {
                Some(d) => d.refresh(true),
                None => disks = Disks::new_with_refreshed_list().ok(),
            }
            if let Some(d) = disks.as_ref() {
                disk_cache = collect_disks(d);
            }
        }

        let now_secs = started.elapsed().as_secs_f64();
        let memory = memory_metrics(sys.total_memory(), sys.available_memory());

        // Commit/pagefile data is Windows-only and optional: pressure degrades to RAM-only.
        let commit = read_commit_info();
        let pressure = lock(&state.pressure).update(
            PressureInputs {
                percent: memory.percent,
                available_bytes: memory.available_bytes,
                total_bytes: memory.total_bytes,
                commit_percent: commit.map(|c| c.commit_percent()),
                pagefile_percent: commit.map(|c| c.pagefile_overflow_percent()),
            },
            now_secs,
        );

        let telemetry = lock(&state.telemetry).update(TelemetryInputs {
            total_bytes: memory.total_bytes,
            available_bytes: memory.available_bytes,
            commit,
            perf: perf.sample(),
            pressure: &pressure,
        });

        lock(&state.growth).observe(now_secs, &process_list);
        let disk_health = lock(&state.disks).update(now_secs, &disk_cache);

        let snapshot = SystemSnapshot {
            timestamp_ms: now_ms(),
            memory,
            cpu_percent: clamp_cpu(sys.global_cpu_usage()),
            disks: disk_cache.clone(),
            disk_health,
            pressure: Some(pressure),
            telemetry: Some(telemetry),
            uptime_secs: System::uptime().unwrap_or(0),
            process_count: sys.processes().len(),
        };
        let workloads = crate::developers::workloads::detect_workloads(&process_list);
        if let Some(history) = state.history.get() {
            history.sample(crate::history::sample_row(&snapshot, &workloads));
        }
        let sessions = record_sessions(&state, &snapshot, &workloads, &process_list, now_secs);
        record_incident(&app, &state, &snapshot, &workloads, &process_list, sessions, now_secs);
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

pub const EVENT_INCIDENT_STARTED: &str = "incident-started";
pub const EVENT_INCIDENT_FINISHED: &str = "incident-finished";

/// Build the recorder's view of this tick: counters, top processes, workload and Docker/WSL state.
pub fn incident_sample(
    snapshot: &SystemSnapshot,
    workloads: &[crate::developers::workloads::Workload],
    processes: &[crate::processes::model::ProcessInfo],
    sessions: Vec<String>,
) -> IncidentSample {
    let t = snapshot.telemetry.as_ref();
    let mut top: Vec<&crate::processes::model::ProcessInfo> = processes.iter().collect();
    top.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes));
    IncidentSample {
        ts_ms: snapshot.timestamp_ms,
        ram_percent: snapshot.memory.percent,
        available_bytes: snapshot.memory.available_bytes,
        used_bytes: snapshot.memory.used_bytes,
        commit_bytes: t.and_then(|t| t.commit_bytes),
        commit_percent: t.and_then(|t| t.commit_percent),
        pagefile_percent: t.and_then(|t| t.pagefile_usage_percent),
        hard_faults_per_sec: t.and_then(|t| t.hard_faults_per_sec),
        cpu_percent: snapshot.cpu_percent,
        disk_active_percent: t.and_then(|t| t.disk_active_percent),
        pressure: snapshot
            .pressure
            .as_ref()
            .map_or(PressureState::Normal, |p| p.state),
        top_processes: top
            .into_iter()
            .take(5)
            .map(|p| NamedBytes {
                name: p.name.clone(),
                bytes: p.memory_bytes,
            })
            .collect(),
        workloads: workloads
            .iter()
            .map(|w| NamedBytes {
                name: w.name.clone(),
                bytes: w.memory_bytes,
            })
            .collect(),
        docker_wsl_bytes: workloads
            .iter()
            .find(|w| matches!(w.kind, crate::developers::workloads::WorkloadKind::Docker))
            .map(|w| w.memory_bytes),
        active_sessions: sessions,
    }
}

fn record_incident(
    app: &AppHandle,
    state: &MonitorState,
    snapshot: &SystemSnapshot,
    workloads: &[crate::developers::workloads::Workload],
    processes: &[crate::processes::model::ProcessInfo],
    sessions: Vec<String>,
    now_secs: f64,
) {
    let sample = incident_sample(snapshot, workloads, processes, sessions);
    let escalating = snapshot
        .pressure
        .as_ref()
        .is_some_and(|p| p.trend == Trend::Escalating);
    let (was_active, finished, is_active) = {
        let mut rec = lock(&state.incident);
        let was = rec.is_active();
        let done = rec.observe(now_secs, sample, escalating);
        (was, done, rec.is_active())
    };
    if !was_active && is_active {
        let _ = app.emit(EVENT_INCIDENT_STARTED, snapshot.timestamp_ms);
    }
    if let Some(summary) = finished {
        let _ = app.emit(EVENT_INCIDENT_FINISHED, &summary);
        if let Some(h) = state.history.get() {
            h.incident(summary);
        }
    }
}

/// Advance the build/session tracker; finished sessions go to history. Returns the active labels.
fn record_sessions(
    state: &MonitorState,
    snapshot: &SystemSnapshot,
    workloads: &[crate::developers::workloads::Workload],
    processes: &[crate::processes::model::ProcessInfo],
    now_secs: f64,
) -> Vec<String> {
    use crate::developers::sessions::{Tick, detect};
    let detected = detect(processes);
    let mut tracker = lock(&state.sessions);
    // Nothing running and nothing detected: skip building the per-tick input.
    if detected.is_empty() && tracker.active_labels().is_empty() {
        tracker.observe(
            now_secs,
            &Tick {
                ts_ms: snapshot.timestamp_ms,
                ram_used_bytes: snapshot.memory.used_bytes,
                ram_percent: snapshot.memory.percent,
                cpu_percent: snapshot.cpu_percent,
                disk_active_percent: None,
                workloads: &[],
                docker_bytes: None,
                pids: &[],
            },
            &detected,
        );
        return Vec::new();
    }
    let named: Vec<NamedBytes> = workloads
        .iter()
        .map(|w| NamedBytes {
            name: w.name.clone(),
            bytes: w.memory_bytes,
        })
        .collect();
    let pids: Vec<u32> = processes.iter().map(|p| p.pid).collect();
    let docker = workloads
        .iter()
        .find(|w| matches!(w.kind, crate::developers::workloads::WorkloadKind::Docker))
        .map(|w| w.memory_bytes);
    let finished = tracker.observe(
        now_secs,
        &Tick {
            ts_ms: snapshot.timestamp_ms,
            ram_used_bytes: snapshot.memory.used_bytes,
            ram_percent: snapshot.memory.percent,
            cpu_percent: snapshot.cpu_percent,
            disk_active_percent: snapshot
                .telemetry
                .as_ref()
                .and_then(|t| t.disk_active_percent),
            workloads: &named,
            docker_bytes: docker,
            pids: &pids,
        },
        &detected,
    );
    let labels = tracker.active_labels();
    drop(tracker);
    if let Some(h) = state.history.get() {
        for s in finished {
            h.session(s);
        }
    }
    labels
}
