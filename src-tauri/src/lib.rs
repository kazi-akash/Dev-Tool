mod commands;
mod developers;
mod diagnosis;
mod docker;
mod monitoring;
mod processes;
mod storage;
mod util;
mod windows;

use std::sync::Arc;

use tauri::Manager;

use docker::DockerMonitor;
use monitoring::service::MonitorState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second launch must not create a second tray icon; focus the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::window::show_main_window(app);
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
        ])
        .setup(|app| {
            let state = Arc::new(MonitorState::default());
            app.manage(state.clone());
            app.manage(Arc::new(DockerMonitor::default()));
            app.manage(Arc::new(storage::state::StorageState::default()));
            monitoring::service::start(app.handle().clone(), state);

            windows::tray::setup(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
