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
