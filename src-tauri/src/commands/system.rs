use std::sync::Arc;

use tauri::State;

use crate::monitoring::disks::DiskThresholds;
use crate::monitoring::growth::GrowthFinding;
use crate::monitoring::metrics::SystemSnapshot;
use crate::monitoring::pressure::PressureThresholds;
use crate::monitoring::service::{MonitorState, lock};

type Monitor<'a> = State<'a, Arc<MonitorState>>;

/// Latest sample, or `None` before the first tick completes.
#[tauri::command]
pub fn get_system_snapshot(state: Monitor<'_>) -> Option<SystemSnapshot> {
    state.latest()
}

/// Change the sampling interval. Rejected unless within the allowed range.
#[tauri::command]
pub fn set_monitor_interval(state: Monitor<'_>, secs: u64) -> Result<(), String> {
    state.interval.set_secs(secs)
}

#[tauri::command]
pub fn get_pressure_thresholds(state: Monitor<'_>) -> PressureThresholds {
    lock(&state.pressure).thresholds()
}

#[tauri::command]
pub fn set_pressure_thresholds(
    state: Monitor<'_>,
    moderate: f32,
    high: f32,
    critical: f32,
) -> Result<PressureThresholds, String> {
    let t = PressureThresholds::validated(moderate, high, critical)?;
    lock(&state.pressure).set_thresholds(t);
    Ok(t)
}

#[tauri::command]
pub fn get_disk_thresholds(state: Monitor<'_>) -> DiskThresholds {
    lock(&state.disks).thresholds()
}

#[tauri::command]
pub fn set_disk_thresholds(
    state: Monitor<'_>,
    warning_free: f32,
    high_free: f32,
    critical_free: f32,
) -> Result<DiskThresholds, String> {
    let t = DiskThresholds::validated(warning_free, high_free, critical_free)?;
    lock(&state.disks).set_thresholds(t);
    Ok(t)
}

/// Processes whose memory has grown steadily (possible leaks), largest growth first.
#[tauri::command]
pub fn get_memory_growth(state: Monitor<'_>) -> Vec<GrowthFinding> {
    lock(&state.growth).findings()
}
