// `unsafe` is confined to two audited Windows modules: `windows/memory.rs` and `windows/perf.rs`.
#![deny(unsafe_code)]

mod actions;
mod ai;
mod commands;
mod database;
mod developers;
mod diagnosis;
mod docker;
mod history;
mod monitoring;
mod processes;
mod storage;
mod util;
mod windows;
#[cfg(test)]
mod workflow_tests;

use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;

use actions::automation::ActionsState;
use docker::DockerMonitor;
use monitoring::service::MonitorState;
use storage::state::StorageState;

/// How often a background diagnosis is stored in history (the UI can run its own any time).
const DIAGNOSIS_RECORD_EVERY: Duration = Duration::from_secs(120);

/// Open the history database and start recording. Any failure leaves history off and the app running.
fn start_history(
    app: &tauri::App,
    monitor: &Arc<MonitorState>,
    actions: &Arc<ActionsState>,
    docker: &Arc<DockerMonitor>,
    storage: &Arc<StorageState>,
) {
    let path = match app.path().app_data_dir() {
        Ok(dir) => dir.join("devpilot.db"),
        Err(e) => {
            eprintln!("devpilot: no data folder, history disabled: {e}");
            return;
        }
    };
    let db = match tauri::async_runtime::block_on(database::Database::open(&path)) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("devpilot: history database unavailable, continuing without it: {e}");
            return;
        }
    };
    let handle = history::start(db);
    // From here on the sampler records every tick into history.
    let _ = monitor.history.set(handle.clone());

    let sink = handle.clone();
    actions
        .engine
        .set_sink(move |rec| sink.action(history::stored_action(rec)));

    let (monitor, docker, storage) = (monitor.clone(), docker.clone(), storage.clone());
    let recorder = handle.clone();
    tauri::async_runtime::spawn(async move {
        // Let the monitor produce its first samples before the first diagnosis.
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut tick = tokio::time::interval(DIAGNOSIS_RECORD_EVERY);
        loop {
            tick.tick().await;
            let (m, d, s) = (monitor.clone(), docker.clone(), storage.clone());
            let gathered = tauri::async_runtime::spawn_blocking(move || {
                commands::diagnosis::gather(&m, &d, &s)
            })
            .await;
            if let Ok(Ok(g)) = gathered {
                recorder.diagnosis(history::stored_diagnosis(&g.report));
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second launch must not create a second tray icon; focus the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::window::show_main_window(app, None);
        }))
        .on_window_event(windows::window::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::system::get_system_snapshot,
            commands::system::set_monitor_interval,
            commands::system::get_pressure_thresholds,
            commands::system::set_pressure_thresholds,
            commands::system::get_disk_thresholds,
            commands::system::set_disk_thresholds,
            commands::system::get_memory_growth,
            commands::processes::get_processes,
            commands::developers::get_workloads,
            commands::developers::get_docker_status,
            commands::storage::start_storage_scan,
            commands::storage::cancel_storage_scan,
            commands::storage::get_storage_scan,
            commands::diagnosis::run_diagnosis,
            commands::actions::list_actions,
            commands::actions::preview_action,
            commands::actions::execute_action,
            commands::actions::rollback_action,
            commands::actions::get_action_history,
            commands::actions::get_safe_automation,
            commands::actions::set_safe_automation,
            commands::ai::ai_key_status,
            commands::ai::set_ai_key,
            commands::ai::clear_ai_key,
            commands::ai::preview_ai_payload,
            commands::ai::ask_ai,
            commands::history::get_history,
            commands::history::get_recent_diagnoses,
            commands::history::get_recent_actions,
            commands::history::get_incidents,
            commands::history::get_incident_status,
            commands::history::get_build_sessions,
            commands::window::take_pending_route,
        ])
        .setup(|app| {
            let monitor = Arc::new(MonitorState::default());
            let docker = Arc::new(DockerMonitor::default());
            let storage = Arc::new(StorageState::default());
            let actions = Arc::new(ActionsState::default());

            app.manage(monitor.clone());
            app.manage(docker.clone());
            app.manage(storage.clone());
            app.manage(actions.clone());
            app.manage(monitor.history.clone());
            app.manage(windows::window::PendingRoute::default());
            app.manage(Arc::new(ai::keystore::CredentialManager::new())
                as Arc<dyn ai::keystore::SecretStore>);

            start_history(app, &monitor, &actions, &docker, &storage);
            actions::automation::start(actions, monitor.clone());
            monitoring::service::start(app.handle().clone(), monitor);

            windows::tray::setup(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Closing the last window must not quit: DevPilot keeps monitoring from the tray.
            // An explicit Quit (tray menu) carries an exit code and is allowed through.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event
                && code.is_none()
            {
                api.prevent_exit();
            }
        });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// Command names registered with the invoke handler in this file.
    fn handler_commands() -> BTreeSet<String> {
        include_str!("lib.rs")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("commands::"))
            .filter_map(|l| l.strip_suffix(','))
            .filter_map(|l| l.rsplit("::").next())
            .map(str::to_string)
            .collect()
    }

    /// Names declared in build.rs (the ACL manifest).
    fn manifest_commands() -> BTreeSet<String> {
        let src = include_str!("../build.rs");
        let start = src.find("const COMMANDS").expect("COMMANDS list");
        let body = &src[start
            ..src[start..]
                .find("];")
                .map(|e| start + e)
                .expect("end of list")];
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    /// `allow-xyz-abc` permissions in the capability file, mapped back to command names.
    fn granted_commands() -> BTreeSet<String> {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).expect("valid json");
        v["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .filter_map(|p| p.as_str())
            .filter_map(|p| p.strip_prefix("allow-"))
            .map(|p| p.replace('-', "_"))
            .collect()
    }

    #[test]
    fn invoke_handler_manifest_and_capability_agree() {
        let handler = handler_commands();
        assert!(
            handler.len() >= 30,
            "found only {} handler commands: {handler:?}",
            handler.len()
        );
        assert_eq!(
            handler,
            manifest_commands(),
            "build.rs COMMANDS must match the invoke handler"
        );
        assert_eq!(
            handler,
            granted_commands(),
            "capabilities/default.json must grant exactly the handler commands"
        );
    }

    #[test]
    fn frontend_gets_no_ambient_powers() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let perms: Vec<&str> = v["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        for p in &perms {
            assert!(
                p.starts_with("allow-")
                    || matches!(*p, "core:event:allow-listen" | "core:event:allow-unlisten"),
                "unexpected permission {p}"
            );
        }
        let windows: Vec<&str> = v["windows"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|w| w.as_str())
            .collect();
        assert_eq!(windows, vec!["main"]);
    }

    #[test]
    fn csp_is_strict_in_release() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let csp = conf["app"]["security"]["csp"]
            .as_str()
            .expect("a release CSP must be set");
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        let script = csp
            .split(';')
            .find(|d| d.trim_start().starts_with("script-src"))
            .unwrap();
        assert!(
            !script.contains("unsafe-inline")
                && !script.contains("unsafe-eval")
                && !script.contains('*'),
            "{script}"
        );
        assert!(
            !csp.contains("http://") || csp.contains("http://ipc.localhost"),
            "no remote http origins in release"
        );
    }
}
