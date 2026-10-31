//! System tray: menu, left-click to open, route navigation events to the frontend.

use tauri::{
    AppHandle, Emitter,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use super::window::show_main_window;

pub const MENU_OPEN: &str = "open";
pub const MENU_STATUS: &str = "status";
pub const MENU_SCAN_STORAGE: &str = "scan_storage";
pub const MENU_DIAGNOSIS: &str = "diagnosis";
pub const MENU_SETTINGS: &str = "settings";
pub const MENU_QUIT: &str = "quit";

/// Frontend route opened by a tray menu entry, if it opens one.
pub fn route_for(menu_id: &str) -> Option<&'static str> {
    match menu_id {
        MENU_OPEN | MENU_STATUS => Some("/"),
        MENU_SCAN_STORAGE => Some("/storage"),
        MENU_DIAGNOSIS => Some("/diagnosis"),
        MENU_SETTINGS => Some("/settings"),
        _ => None,
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open DevPilot", true, None::<&str>)?;
    let status = MenuItem::with_id(app, MENU_STATUS, "System Status", true, None::<&str>)?;
    let scan = MenuItem::with_id(app, MENU_SCAN_STORAGE, "Scan Storage", true, None::<&str>)?;
    let diagnosis = MenuItem::with_id(app, MENU_DIAGNOSIS, "Run Diagnosis", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let sep_top = PredefinedMenuItem::separator(app)?;
    let sep_bottom = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &open,
            &sep_top,
            &status,
            &scan,
            &diagnosis,
            &settings,
            &sep_bottom,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("DevPilot")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == MENU_QUIT {
                app.exit(0);
                return;
            }
            if let Some(route) = route_for(id) {
                show_main_window(app);
                // Best effort: the frontend may not be listening yet during startup.
                let _ = app.emit("navigate", route);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_entries_map_to_routes() {
        assert_eq!(route_for(MENU_OPEN), Some("/"));
        assert_eq!(route_for(MENU_STATUS), Some("/"));
        assert_eq!(route_for(MENU_SCAN_STORAGE), Some("/storage"));
        assert_eq!(route_for(MENU_DIAGNOSIS), Some("/diagnosis"));
        assert_eq!(route_for(MENU_SETTINGS), Some("/settings"));
    }

    #[test]
    fn quit_and_unknown_have_no_route() {
        assert_eq!(route_for(MENU_QUIT), None);
        assert_eq!(route_for("../../etc"), None);
        assert_eq!(route_for(""), None);
    }
}
