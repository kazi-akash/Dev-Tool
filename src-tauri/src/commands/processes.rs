use std::sync::Arc;

use tauri::State;

use crate::monitoring::service::MonitorState;
use crate::processes::model::{ProcessList, ProcessQuery};

/// Filtered, sorted, size-capped view of the live process list.
#[tauri::command]
pub fn get_processes(state: State<'_, Arc<MonitorState>>, query: ProcessQuery) -> ProcessList {
    state.processes.query(&query)
}

/// Parent/child tree of the live process list: the biggest subtrees only, depth and size bounded.
/// Built on request from the cached list, so it adds nothing to the sampler.
#[tauri::command]
pub fn get_process_tree(
    state: State<'_, Arc<MonitorState>>,
) -> crate::processes::tree::ProcessTree {
    crate::processes::tree::build_tree(&state.processes.snapshot())
}
