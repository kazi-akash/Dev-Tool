// `unsafe` is confined to three audited Windows modules: `windows/memory.rs`, `windows/perf.rs` and
// `windows/priority.rs`.
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
mod prefs;
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
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second *autostart* launch must stay quiet; a manual launch shows the window.
            if !windows::autostart::started_minimized(args) {
                windows::window::show_main_window(app, None);
            }
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
            commands::system::set_notifications_enabled,
            commands::system::get_notifications_enabled,
            commands::system::send_test_notification,
            commands::system::get_start_with_windows,
            commands::system::set_start_with_windows,
            commands::processes::get_processes,
            commands::processes::get_process_tree,
            commands::developers::get_workloads,
            commands::developers::get_docker_status,
            commands::developers::get_docker_breakdown,
            commands::emergency::get_emergency_status,
            commands::quick::get_quick_status,
            commands::quick::quick_status_action,
            commands::storage::start_storage_scan,
            commands::storage::start_deep_appdata_scan,
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
            app.manage(windows::quick::QuickState::default());
            app.manage(Arc::new(ai::keystore::CredentialManager::new())
                as Arc<dyn ai::keystore::SecretStore>);

            // Preferences the Rust side owns are loaded here, so a minimized start (no window to
            // push them from the UI) still honours the user's settings.
            let prefs = Arc::new(prefs::PrefsStore::new(
                app.path().app_data_dir().ok().map(|d| d.join("prefs.json")),
            ));
            app.manage(prefs.clone());

            // The storage scan yields to pressure, builds and incidents (see storage/governor.rs).
            {
                let m = monitor.clone();
                storage.set_governor(storage::governor::GovernorHandle::new(move || {
                    storage::governor::decide(&storage::governor::Conditions {
                        pressure: m.latest().and_then(|s| s.pressure.map(|p| p.state)),
                        active_sessions: monitoring::service::lock(&m.sessions).active_labels(),
                        incident_active: monitoring::service::lock(&m.incident).is_active(),
                    })
                }));
            }

            // Toasts: one notifier shared by the sampler, scans and actions.
            let notifier = Arc::new(monitoring::notifier::Notifier::new(Box::new(
                windows::toast::SystemToaster::new(app.handle().clone()),
            )));
            let _ = monitor.notifier.set(notifier.clone());
            {
                let n = notifier.clone();
                storage.set_on_done(move |r| {
                    let total: u64 = r.entries.iter().map(|e| e.size_bytes).sum();
                    n.result(
                        &monitoring::alerts::scan_alert(
                            r.entries.len(),
                            total,
                            r.complete,
                            r.started_at_ms,
                        ),
                        r.started_at_ms + r.duration_ms,
                    );
                });
            }
            {
                // One sink for everything that follows an action: history (when the database
                // opened) and the completion toast. History is looked up lazily, so this can be
                // registered before the database exists.
                let (n, m) = (notifier.clone(), monitor.clone());
                actions.engine.set_sink(move |rec| {
                    if let Some(h) = m.history.get() {
                        h.action(history::stored_action(rec));
                    }
                    n.result(
                        &monitoring::alerts::action_alert(
                            rec.id,
                            &rec.name,
                            rec.success,
                            &rec.summary,
                            rec.automated,
                        ),
                        rec.at_ms,
                    );
                });
            }

            prefs::apply(&prefs.snapshot(), &monitor, &actions.automation);

            start_history(app, &monitor, &docker, &storage);
            actions::automation::start(actions, monitor.clone());
            monitoring::service::start(app.handle().clone(), monitor);

            windows::tray::setup(app.handle())?;
            // The window is not created automatically (`create: false`): a normal launch opens it,
            // a start with Windows (`--minimized`) leaves only the tray and the monitor running.
            if !windows::autostart::started_minimized(std::env::args()) {
                windows::window::show_main_window(app.handle(), None);
            }
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
    fn granted_in(json: &str) -> BTreeSet<String> {
        let v: serde_json::Value = serde_json::from_str(json).expect("valid json");
        v["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .filter_map(|p| p.as_str())
            .filter_map(|p| p.strip_prefix("allow-"))
            .map(|p| p.replace('-', "_"))
            .collect()
    }

    /// Commands the main window may call.
    fn granted_commands() -> BTreeSet<String> {
        granted_in(include_str!("../capabilities/default.json"))
    }

    /// Commands the tray status popup may call.
    fn popup_commands() -> BTreeSet<String> {
        granted_in(include_str!("../capabilities/status.json"))
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
            granted_commands()
                .union(&popup_commands())
                .cloned()
                .collect::<BTreeSet<_>>(),
            "the capability files together must grant exactly the handler commands"
        );
    }

    /// The popup window is a separate, tiny capability: two commands, no events, and nothing it
    /// grants is also granted to the main window (so neither can use the other's powers).
    #[test]
    fn status_popup_capability_is_minimal() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/status.json")).unwrap();
        assert_eq!(v["windows"], serde_json::json!(["status"]));
        assert_eq!(
            popup_commands(),
            ["get_quick_status", "quick_status_action"]
                .into_iter()
                .map(String::from)
                .collect::<BTreeSet<_>>()
        );
        for p in v["permissions"].as_array().unwrap() {
            assert!(p.as_str().unwrap().starts_with("allow-"), "{p}");
        }
        assert!(
            granted_commands().is_disjoint(&popup_commands()),
            "main window and popup must not share commands"
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

    /// Production part of a source file (everything before its test module).
    fn production(src: &str) -> &str {
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    /// AI output is untrusted text. The AI gateway and its command layer must have no route into the
    /// action engine, the command runner or process creation: the only path from a recommendation to
    /// an effect is a deterministic action id, Rust validation, the permission check, the preview, the
    /// user's approval, execution and verification, all inside `actions/`.
    #[test]
    fn ai_code_has_no_path_to_actions_or_process_execution() {
        let files = [
            ("ai/mod.rs", include_str!("ai/mod.rs")),
            ("ai/client.rs", include_str!("ai/client.rs")),
            ("ai/keystore.rs", include_str!("ai/keystore.rs")),
            ("ai/payload.rs", include_str!("ai/payload.rs")),
            ("ai/provider.rs", include_str!("ai/provider.rs")),
            ("ai/sanitize.rs", include_str!("ai/sanitize.rs")),
            ("commands/ai.rs", include_str!("commands/ai.rs")),
        ];
        for (name, src) in files {
            let src = production(src);
            for banned in [
                "crate::actions",
                "ActionEngine",
                "ActionsState",
                "execute_automated",
                "std::process::Command",
                "Command::new",
                "run_program",
                "CommandRunner",
                "ProcessKiller",
            ] {
                assert!(
                    !src.contains(banned),
                    "{name} must not reference `{banned}`"
                );
            }
        }
    }

    /// The emergency assessment only *describes* steps (references to action ids). It must not run,
    /// kill, delete or call the action engine itself.
    #[test]
    fn emergency_assessment_never_executes_anything() {
        let src = production(include_str!("monitoring/emergency.rs"));
        for banned in [
            "Command::new",
            "run_program",
            "ActionEngine",
            ".execute(",
            "execute_automated",
            ".kill(",
            "remove_dir_all",
            "remove_file",
        ] {
            assert!(
                !src.contains(banned),
                "emergency.rs must not contain `{banned}`"
            );
        }
        let cmd = production(include_str!("commands/emergency.rs"));
        assert!(
            !cmd.contains("ActionsState") && !cmd.contains("execute"),
            "the emergency command only reads"
        );
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
