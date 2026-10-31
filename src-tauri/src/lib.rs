mod windows;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second launch must not create a second tray icon; focus the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::window::show_main_window(app);
        }))
        .on_window_event(windows::window::on_window_event)
        .setup(|app| {
            windows::tray::setup(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
