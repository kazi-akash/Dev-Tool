use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::State;

use crate::developers::workloads::detect_workloads;
use crate::diagnosis::engine::diagnose;
use crate::diagnosis::model::DiagnosisReport;
use crate::diagnosis::rules::{Config, Input};
use crate::docker::DockerMonitor;
use crate::monitoring::service::{MonitorState, lock};
use crate::storage::state::StorageState;

/// Gather the current facts and run every rule. Docker details come from the CLI, so this runs on a
/// worker thread and degrades to "Docker details unavailable" instead of failing.
#[tauri::command]
pub async fn run_diagnosis(
    monitor: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
    storage: State<'_, Arc<StorageState>>,
) -> Result<DiagnosisReport, String> {
    let monitor = monitor.inner().clone();
    let docker = docker.inner().clone();
    let storage = storage.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = monitor
            .latest()
            .ok_or("Monitoring has not produced a sample yet. Try again in a moment.")?;
        let pressure = snapshot
            .pressure
            .clone()
            .ok_or("Memory pressure is not available yet.")?;
        let workloads = detect_workloads(&monitor.processes.snapshot());
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
        Ok(diagnose(&input, &Config::default(), now))
    })
    .await
    .map_err(|e| e.to_string())?
}
