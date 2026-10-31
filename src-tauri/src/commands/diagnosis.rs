use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::State;

use crate::developers::workloads::{Workload, detect_workloads};
use crate::diagnosis::engine::diagnose;
use crate::diagnosis::model::DiagnosisReport;
use crate::diagnosis::rules::{Config, Input};
use crate::docker::DockerMonitor;
use crate::monitoring::metrics::SystemSnapshot;
use crate::monitoring::service::{MonitorState, lock};
use crate::processes::model::ProcessInfo;
use crate::storage::state::StorageState;

/// Everything one diagnosis run looked at; also feeds the AI payload builder.
pub struct Gathered {
    pub snapshot: SystemSnapshot,
    pub workloads: Vec<Workload>,
    pub processes: Vec<ProcessInfo>,
    pub report: DiagnosisReport,
}

/// Collect the current facts and run every rule. Blocking (Docker CLI): call from a worker thread.
pub fn gather(
    monitor: &MonitorState,
    docker: &DockerMonitor,
    storage: &StorageState,
) -> Result<Gathered, String> {
    let snapshot = monitor
        .latest()
        .ok_or("Monitoring has not produced a sample yet. Try again in a moment.")?;
    let pressure = snapshot
        .pressure
        .clone()
        .ok_or("Memory pressure is not available yet.")?;
    let processes = monitor.processes.snapshot();
    let workloads = detect_workloads(&processes);
    let growth = lock(&monitor.growth).findings();
    let docker_status = docker.status(false);
    let scan = storage.latest_result();

    let input = Input {
        memory: &snapshot.memory,
        pressure: &pressure,
        workloads: &workloads,
        growth: &growth,
        disks: &snapshot.disks,
        disk_health: &snapshot.disk_health,
        docker: Some(&docker_status),
        storage: scan.as_ref().map(|s| s.entries.as_slice()),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let report = diagnose(&input, &Config::default(), now);
    Ok(Gathered {
        snapshot,
        workloads,
        processes,
        report,
    })
}

/// Run the diagnosis. Docker details come from the CLI, so this runs on a worker thread and
/// degrades to "Docker details unavailable" instead of failing.
#[tauri::command]
pub async fn run_diagnosis(
    monitor: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
    storage: State<'_, Arc<StorageState>>,
    history: State<'_, super::history::HistorySlot>,
) -> Result<DiagnosisReport, String> {
    let (monitor, docker, storage) = (
        monitor.inner().clone(),
        docker.inner().clone(),
        storage.inner().clone(),
    );
    let report = tauri::async_runtime::spawn_blocking(move || {
        gather(&monitor, &docker, &storage).map(|g| g.report)
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(h) = history.get() {
        h.diagnosis(crate::history::stored_diagnosis(&report));
    }
    Ok(report)
}
