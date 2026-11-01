use std::sync::Arc;

use tauri::State;

use crate::monitoring::disks::DiskThresholds;
use crate::monitoring::growth::GrowthFinding;
use crate::monitoring::metrics::SystemSnapshot;
use crate::monitoring::pressure::PressureThresholds;
use crate::monitoring::service::{MonitorState, lock};

type Monitor<'a> = State<'a, Arc<MonitorState>>;
type Prefs<'a> = State<'a, Arc<crate::prefs::PrefsStore>>;

/// Latest sample, or `None` before the first tick completes.
#[tauri::command]
pub fn get_system_snapshot(state: Monitor<'_>) -> Option<SystemSnapshot> {
    state.latest()
}

/// Change the sampling interval. Rejected unless within the allowed range.
#[tauri::command]
pub fn set_monitor_interval(state: Monitor<'_>, prefs: Prefs<'_>, secs: u64) -> Result<(), String> {
    state.interval.set_secs(secs)?;
    prefs.update(|p| p.monitor_interval_secs = Some(secs));
    Ok(())
}

#[tauri::command]
pub fn get_pressure_thresholds(state: Monitor<'_>) -> PressureThresholds {
    lock(&state.pressure).thresholds()
}

#[tauri::command]
pub fn set_pressure_thresholds(
    state: Monitor<'_>,
    prefs: Prefs<'_>,
    moderate: f32,
    high: f32,
    critical: f32,
) -> Result<PressureThresholds, String> {
    let t = PressureThresholds::validated(moderate, high, critical)?;
    lock(&state.pressure).set_thresholds(t);
    prefs.update(|p| p.pressure = Some([moderate, high, critical]));
    Ok(t)
}

#[tauri::command]
pub fn get_disk_thresholds(state: Monitor<'_>) -> DiskThresholds {
    lock(&state.disks).thresholds()
}

#[tauri::command]
pub fn set_disk_thresholds(
    state: Monitor<'_>,
    prefs: Prefs<'_>,
    warning_free: f32,
    high_free: f32,
    critical_free: f32,
) -> Result<DiskThresholds, String> {
    let t = DiskThresholds::validated(warning_free, high_free, critical_free)?;
    lock(&state.disks).set_thresholds(t);
    prefs.update(|p| p.disk = Some([warning_free, high_free, critical_free]));
    Ok(t)
}

/// Processes whose memory has grown steadily (possible leaks), largest growth first.
#[tauri::command]
pub fn get_memory_growth(state: Monitor<'_>) -> Vec<GrowthFinding> {
    lock(&state.growth).findings()
}

/// Master switch for toast notifications (the UI setting). Enforced here, not just in the UI.
#[tauri::command]
pub fn set_notifications_enabled(state: Monitor<'_>, prefs: Prefs<'_>, enabled: bool) {
    prefs.update(|p| p.notifications = Some(enabled));
    if let Some(n) = state.notifier.get() {
        n.set_enabled(enabled);
    }
}

#[tauri::command]
pub fn get_notifications_enabled(state: Monitor<'_>) -> bool {
    state.notifier.get().is_none_or(|n| n.enabled())
}

/// Show a harmless toast so the user can see that notifications work on this PC.
#[tauri::command]
pub fn send_test_notification(state: Monitor<'_>) -> Result<(), String> {
    state
        .notifier
        .get()
        .ok_or("Notifications are not ready yet.")?
        .test()
}

/// Whether Windows will start DevPilot at sign-in (read from the registry, not from a saved copy).
#[tauri::command]
pub fn get_start_with_windows() -> Result<crate::windows::autostart::AutostartStatus, String> {
    Ok(crate::windows::autostart::Autostart::for_this_app()?.status())
}

/// The only place the startup entry is written, and only when the user flips the switch.
#[tauri::command]
pub fn set_start_with_windows(
    enabled: bool,
) -> Result<crate::windows::autostart::AutostartStatus, String> {
    let a = crate::windows::autostart::Autostart::for_this_app()?;
    a.set_enabled(enabled)?;
    Ok(a.status())
}
