//! Main window lifecycle.
//!
//! Closing the window destroys its WebView (about 450 MB of WebView2 processes) while the Rust
//! core, tray and monitoring keep running. Opening it again, from the tray or by launching DevPilot
//! a second time, recreates it from the window config.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder, Window, WindowEvent};

pub const MAIN_WINDOW: &str = "main";

/// Routes the tray may ask the UI to open. Anything else is ignored.
const ROUTES: &[&str] = &[
    "/",
    "/processes",
    "/storage",
    "/diagnosis",
    "/actions",
    "/history",
    "/settings",
];

pub fn is_allowed_route(route: &str) -> bool {
    ROUTES.contains(&route)
}

/// A route requested while the window did not exist; the new page claims it on startup.
#[derive(Default)]
pub struct PendingRoute(Mutex<Option<String>>);

impl PendingRoute {
    pub fn set(&self, route: &str) {
        if is_allowed_route(route) {
            *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(route.to_string());
        }
    }

    pub fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Show the main window (recreating it if it was closed) and optionally open a route.
pub fn show_main_window(app: &AppHandle, route: Option<&str>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(r) = route.filter(|r| is_allowed_route(r)) {
            let _ = app.emit("navigate", r);
        }
        return;
    }

    if let Some(r) = route {
        app.state::<PendingRoute>().set(r);
    }
    let Some(config) = app.config().app.windows.first() else {
        return;
    };
    match WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => eprintln!("devpilot: could not reopen the window: {e}"),
    }
}

/// The window may close normally: DevPilot keeps running in the tray (see `RunEvent::ExitRequested`
/// in `lib.rs`), and the destroyed WebView releases its memory.
pub fn on_window_event(_window: &Window, _event: &WindowEvent) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_routes_are_allowed() {
        for r in ROUTES {
            assert!(is_allowed_route(r));
        }
        for bad in [
            "",
            "storage",
            "/storage/../x",
            "http://evil",
            "javascript:alert(1)",
            "/unknown",
            "//evil.com",
        ] {
            assert!(!is_allowed_route(bad), "{bad}");
        }
    }

    #[test]
    fn pending_route_is_taken_once_and_filtered() {
        let p = PendingRoute::default();
        assert_eq!(p.take(), None);
        p.set("https://evil.example");
        assert_eq!(p.take(), None, "unknown routes are never stored");
        p.set("/diagnosis");
        assert_eq!(p.take().as_deref(), Some("/diagnosis"));
        assert_eq!(p.take(), None);
    }
}
