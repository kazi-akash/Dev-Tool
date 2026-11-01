//! Tray quick-status popup: a small undecorated window that exists only while it is open.
//!
//! It reuses the app bundle (route `#/quick-status`) and reads one cached status command, so it
//! adds no monitoring. Like the main window, it is destroyed when closed or when it loses focus,
//! so it costs nothing while hidden.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

pub const QUICK_WINDOW: &str = "status";
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 340.0;
const MARGIN: f64 = 16.0;
/// Space kept free at the bottom for the Windows taskbar.
const TASKBAR: f64 = 56.0;
/// A focus-lost event right after creation is the window still settling, not the user clicking away.
const BLUR_GRACE: Duration = Duration::from_millis(600);

/// When the popup was created; used to ignore the initial blur.
#[derive(Default)]
pub struct QuickState(Mutex<Option<Instant>>);

impl QuickState {
    fn opened(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    fn settled(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|t| t.elapsed() >= BLUR_GRACE)
    }
}

/// Bottom-right corner of the work area, in logical pixels.
pub fn popup_position(monitor_w: f64, monitor_h: f64, origin: (f64, f64)) -> (f64, f64) {
    (
        origin.0 + (monitor_w - WIDTH - MARGIN).max(0.0),
        origin.1 + (monitor_h - HEIGHT - MARGIN - TASKBAR).max(0.0),
    )
}

pub fn show_quick_status(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(QUICK_WINDOW) {
        let _ = w.set_focus();
        return;
    }
    app.state::<QuickState>().opened();
    let mut builder = WebviewWindowBuilder::new(
        app,
        QUICK_WINDOW,
        WebviewUrl::App("index.html#/quick-status".into()),
    )
    .title("DevPilot status")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(true);

    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor().max(1.0);
        let size = m.size();
        let pos = m.position();
        let (x, y) = popup_position(
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
            (f64::from(pos.x) / scale, f64::from(pos.y) / scale),
        );
        builder = builder.position(x, y);
    }
    if let Err(e) = builder.build() {
        eprintln!("devpilot: could not open the status popup: {e}");
    }
}

/// Close the popup when the user clicks elsewhere.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != QUICK_WINDOW {
        return;
    }
    if let WindowEvent::Focused(false) = event
        && window.app_handle().state::<QuickState>().settled()
    {
        let _ = window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_sits_in_the_bottom_right_above_the_taskbar() {
        let (x, y) = popup_position(1920.0, 1080.0, (0.0, 0.0));
        assert_eq!(x, 1920.0 - WIDTH - MARGIN);
        assert_eq!(y, 1080.0 - HEIGHT - MARGIN - TASKBAR);
    }

    #[test]
    fn popup_follows_a_monitor_that_is_not_at_the_origin() {
        let (x, _) = popup_position(1920.0, 1080.0, (1920.0, 0.0));
        assert_eq!(x, 1920.0 + 1920.0 - WIDTH - MARGIN);
    }

    #[test]
    fn tiny_monitors_never_give_negative_positions() {
        let (x, y) = popup_position(200.0, 200.0, (0.0, 0.0));
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn blur_is_ignored_until_the_popup_has_settled() {
        let s = QuickState::default();
        assert!(!s.settled(), "never opened");
        s.opened();
        assert!(!s.settled(), "just opened");
    }
}
