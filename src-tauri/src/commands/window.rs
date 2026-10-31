use tauri::State;

use crate::windows::window::PendingRoute;

/// The route the tray asked for while the window was closed. Returned once, then cleared.
#[tauri::command]
pub fn take_pending_route(pending: State<'_, PendingRoute>) -> Option<String> {
    pending.take()
}
