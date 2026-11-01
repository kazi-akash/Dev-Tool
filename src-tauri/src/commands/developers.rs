use std::sync::Arc;

use tauri::State;

use crate::developers::workloads::{Workload, detect_workloads};
use crate::docker::{DockerMonitor, DockerStatus};
use crate::monitoring::service::MonitorState;

/// Developer workloads (Docker/WSL2, Android/Gradle, Next.js, Laragon...) from the live process list.
#[tauri::command]
pub fn get_workloads(state: State<'_, Arc<MonitorState>>) -> Vec<Workload> {
    detect_workloads(&state.processes.snapshot())
}

/// Running containers with memory/CPU. Runs the Docker CLI on a worker thread; cached for 10 s.
#[tauri::command]
pub async fn get_docker_status(
    docker: State<'_, Arc<DockerMonitor>>,
    force: Option<bool>,
) -> Result<DockerStatus, String> {
    let docker = docker.inner().clone();
    tauri::async_runtime::spawn_blocking(move || docker.status(force.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())
}

/// Docker/WSL2 memory split: host memory, VM, containers and what cannot be attributed.
/// `None` when no Docker/WSL2 processes are running.
#[tauri::command]
pub async fn get_docker_breakdown(
    state: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
) -> Result<Option<crate::docker::attribution::DockerBreakdown>, String> {
    let (state, docker) = (state.inner().clone(), docker.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        let workloads = detect_workloads(&state.processes.snapshot());
        crate::docker::attribution::breakdown(&workloads, &docker.status(false))
    })
    .await
    .map_err(|e| e.to_string())
}
