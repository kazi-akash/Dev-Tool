use std::sync::Arc;

use tauri::State;

use crate::monitoring::service::MonitorState;
use crate::processes::model::{ProcessList, ProcessQuery};

/// Filtered, sorted, size-capped view of the live process list.
#[tauri::command]
pub fn get_processes(state: State<'_, Arc<MonitorState>>, query: ProcessQuery) -> ProcessList {
    state.processes.query(&query)
}
