//! Windows toast notifications. No `unsafe`: the WinRT calls live inside `tauri-winrt-notification`.
//!
//! A click on the toast, or on its button, opens the main window on one of the fixed in-app routes.
//! The activation argument comes from outside the process, so it is checked against the route
//! allow-list again here and anything else falls back to the alert's own destination.

use tauri::{AppHandle, Manager};

#[cfg(windows)]
use super::window::show_main_window;
use super::window::{MAIN_WINDOW, is_allowed_route};
use crate::monitoring::alerts::Alert;
use crate::monitoring::notifier::Toaster;

pub struct SystemToaster {
    app: AppHandle,
}

impl SystemToaster {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// Route to open for an activation argument, or the fallback when the argument is not allowed.
pub fn route_for_activation(arg: Option<&str>, fallback: &'static str) -> String {
    match arg {
        Some(a) if is_allowed_route(a) => a.to_string(),
        _ => fallback.to_string(),
    }
}

/// An installed app has its own toast identity. A build that is run from a build folder is not
/// registered with Windows, so it borrows PowerShell's identity or Windows would drop the toast.
#[cfg(windows)]
fn app_id(app: &AppHandle) -> String {
    let from_build_dir = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_lowercase().contains("\\target\\"))
        .unwrap_or(true);
    if tauri::is_dev() || from_build_dir {
        tauri_winrt_notification::Toast::POWERSHELL_APP_ID.to_string()
    } else {
        app.config().identifier.clone()
    }
}

impl Toaster for SystemToaster {
    #[cfg(windows)]
    fn show(&self, alert: &Alert) -> Result<(), String> {
        use tauri_winrt_notification::Toast;

        let app = self.app.clone();
        let id = app_id(&self.app);
        let (title, body) = (alert.title.clone(), alert.body.clone());
        let route = alert.destination.route();
        let button = alert.destination.button();
        // Showing can take a moment; never on the sampler thread.
        std::thread::Builder::new()
            .name("devpilot-toast".into())
            .spawn(move || {
                let result = Toast::new(&id)
                    .title(&title)
                    .text1(&body)
                    .add_button(button, route)
                    .on_activated(move |arg| {
                        show_main_window(&app, Some(&route_for_activation(arg.as_deref(), route)));
                        Ok(())
                    })
                    .show();
                if let Err(e) = result {
                    eprintln!("devpilot: toast failed: {e}");
                }
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[cfg(not(windows))]
    fn show(&self, _alert: &Alert) -> Result<(), String> {
        Err("toast notifications are only available on Windows".into())
    }

    fn window_visible(&self) -> bool {
        self.app
            .get_webview_window(MAIN_WINDOW)
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_arguments_are_validated() {
        assert_eq!(
            route_for_activation(Some("/history"), "/diagnosis"),
            "/history"
        );
        assert_eq!(route_for_activation(None, "/diagnosis"), "/diagnosis");
        for bad in [
            "",
            "http://evil",
            "/../x",
            "javascript:1",
            "//evil.com",
            "/unknown",
        ] {
            assert_eq!(
                route_for_activation(Some(bad), "/diagnosis"),
                "/diagnosis",
                "{bad}"
            );
        }
    }
}

#[cfg(all(test, windows))]
mod display_tests {
    /// Shows one real toast (as PowerShell, since a test binary is not installed). Run it by hand:
    /// `cargo test --lib real_toast -- --ignored`.
    #[test]
    #[ignore = "displays a real notification"]
    fn real_toast_is_accepted_by_windows() {
        use tauri_winrt_notification::Toast;
        Toast::new(Toast::POWERSHELL_APP_ID)
            .title("DevPilot test notification")
            .text1("Notifications can be displayed on this PC. You can ignore this.")
            .add_button("View diagnosis", "/diagnosis")
            .on_activated(|_| Ok(()))
            .show()
            .expect("Windows accepted the toast");
    }
}
