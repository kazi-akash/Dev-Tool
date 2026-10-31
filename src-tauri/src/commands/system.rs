use std::sync::Arc;

use tauri::State;

use crate::monitoring::metrics::SystemSnapshot;
use crate::monitoring::service::MonitorState;

/// Latest sample, or `None` before the first tick completes.
#[tauri::command]
pub fn get_system_snapshot(state: State<'_, Arc<MonitorState>>) -> Option<SystemSnapshot> {
    state.latest()
}

/// Change the sampling interval. Rejected unless within the allowed range.
#[tauri::command]
pub fn set_monitor_interval(state: State<'_, Arc<MonitorState>>, secs: u64) -> Result<(), String> {
    state.interval.set_secs(secs)
}
