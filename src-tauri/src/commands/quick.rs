use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::developers::workloads::detect_workloads;
use crate::monitoring::quick_status::{QuickStatus, quick_status};
use crate::monitoring::service::{MonitorState, lock};
use crate::windows::quick::QUICK_WINDOW;
use crate::windows::tray;

/// Status for the tray popup. Reads only the cached snapshot and process list: no Docker, no scan.
#[tauri::command]
pub fn get_quick_status(monitor: State<'_, Arc<MonitorState>>) -> Option<QuickStatus> {
    let snapshot = monitor.latest()?;
    let workloads = detect_workloads(&monitor.processes.snapshot());
    let incident = lock(&monitor.incident).is_active();
    Some(quick_status(&snapshot, &workloads, incident))
}

/// The tray menu entry an allowed popup action stands for. Anything else is rejected, so the popup
/// can open pages and quit, and nothing more.
pub fn menu_id_for(action: &str) -> Option<&'static str> {
    match action {
        "open" => Some(tray::MENU_OPEN),
        "diagnosis" => Some(tray::MENU_DIAGNOSIS),
        "scan_storage" => Some(tray::MENU_SCAN_STORAGE),
        "settings" => Some(tray::MENU_SETTINGS),
        "quit" => Some(tray::MENU_QUIT),
        _ => None,
    }
}

/// Run one of the popup's buttons. The popup closes first so it never lingers over the page it opens.
#[tauri::command]
pub fn quick_status_action(app: AppHandle, action: String) -> Result<(), String> {
    let id = menu_id_for(&action).ok_or_else(|| format!("unknown action '{action}'"))?;
    if let Some(w) = app.get_webview_window(QUICK_WINDOW) {
        let _ = w.close();
    }
    tray::run_menu_action(&app, id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_five_popup_buttons_map_to_actions() {
        assert_eq!(menu_id_for("open"), Some("open"));
        assert_eq!(menu_id_for("diagnosis"), Some("diagnosis"));
        assert_eq!(menu_id_for("scan_storage"), Some("scan_storage"));
        assert_eq!(menu_id_for("settings"), Some("settings"));
        assert_eq!(menu_id_for("quit"), Some("quit"));
    }

    #[test]
    fn everything_else_is_rejected() {
        for bad in [
            "",
            "status",
            "execute_action",
            "../open",
            "OPEN",
            "cmd /c calc",
            "wsl.shutdown",
        ] {
            assert_eq!(menu_id_for(bad), None, "{bad}");
        }
    }
}
