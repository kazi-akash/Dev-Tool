use std::sync::Arc;

use tauri::State;

use crate::developers::workloads::detect_workloads;
use crate::docker::DockerMonitor;
use crate::monitoring::emergency::{EmergencyInput, EmergencyReport, assess};
use crate::monitoring::pressure::PressureState;
use crate::monitoring::service::{MonitorState, lock};

/// The emergency picture, or `None` when pressure is not CRITICAL. Cheap when calm: the Docker CLI
/// is only consulted (and cached for 10 s) once pressure really is critical. The report only
/// *references* actions; running one still goes through preview, confirmation and verification.
#[tauri::command]
pub async fn get_emergency_status(
    monitor: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
) -> Result<Option<EmergencyReport>, String> {
    let (monitor, docker) = (monitor.inner().clone(), docker.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = monitor.latest()?;
        if snapshot.pressure.as_ref()?.state != PressureState::Critical {
            return None;
        }
        let processes = monitor.processes.snapshot();
        let workloads = detect_workloads(&processes);
        let docker_status = docker.status(false);
        let (incident_active, sessions) = (
            lock(&monitor.incident).is_active(),
            lock(&monitor.sessions).active_labels(),
        );
        assess(&EmergencyInput {
            snapshot: &snapshot,
            workloads: &workloads,
            processes: &processes,
            docker: Some(&docker_status),
            incident_active,
            active_sessions: sessions,
        })
    })
    .await
    .map_err(|e| e.to_string())
}
