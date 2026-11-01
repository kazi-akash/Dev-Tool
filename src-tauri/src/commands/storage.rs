use std::sync::Arc;

use tauri::State;

use crate::storage::scanner::{ScanEnv, ScanOptions};
use crate::storage::state::{ScanStatus, StorageState};

/// Start a background, read-only storage scan. Returns false if one is already running.
#[tauri::command]
pub fn start_storage_scan(state: State<'_, Arc<StorageState>>) -> bool {
    state
        .inner()
        .start(ScanEnv::from_system(), ScanOptions::default())
}

#[tauri::command]
pub fn cancel_storage_scan(state: State<'_, Arc<StorageState>>) {
    state.cancel();
}

#[tauri::command]
pub fn get_storage_scan(state: State<'_, Arc<StorageState>>) -> ScanStatus {
    state.status()
}

/// Start the optional deep AppData scan. Only runs when the user explicitly asks for it: nothing
/// else calls it, and it is never part of a normal scan. Report-only; it can never delete anything.
#[tauri::command]
pub fn start_deep_appdata_scan(state: State<'_, Arc<StorageState>>) -> bool {
    state
        .inner()
        .start_deep(ScanEnv::from_system(), ScanOptions::default())
}
