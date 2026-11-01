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
    "set_notifications_enabled",
    "get_notifications_enabled",
    "send_test_notification",
    "get_start_with_windows",
    "set_start_with_windows",
    "get_processes",
    "get_process_tree",
    "get_workloads",
    "get_docker_status",
    "get_docker_breakdown",
    "get_emergency_status",
    "get_quick_status",
    "quick_status_action",
    "start_storage_scan",
    "start_deep_appdata_scan",
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
    "get_build_sessions",
    "take_pending_route",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri build script");
}
