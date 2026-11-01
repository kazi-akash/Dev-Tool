//! Declares every command the frontend may call. With an app manifest, Tauri denies any command
//! that is not listed here AND granted in `capabilities/default.json`. A unit test in `lib.rs`
//! keeps this list, the invoke handler, and the capability file identical.

const COMMANDS: &[&str] = &[
    "get_system_snapshot",
    "set_monitor_interval",
    "get_pressure_thresholds",
    "set_pressure_thresholds",
    "get_disk_thresholds",
    "set_disk_thresholds",
    "get_memory_growth",
    "get_processes",
    "get_workloads",
    "get_docker_status",
    "start_storage_scan",
    "cancel_storage_scan",
    "get_storage_scan",
    "run_diagnosis",
    "list_actions",
    "preview_action",
    "execute_action",
    "rollback_action",
    "get_action_history",
    "get_safe_automation",
    "set_safe_automation",
    "ai_key_status",
    "set_ai_key",
    "clear_ai_key",
    "preview_ai_payload",
    "ask_ai",
    "get_history",
    "get_recent_diagnoses",
    "get_recent_actions",
    "get_incidents",
    "get_incident_status",
    "take_pending_route",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri build script");
}
