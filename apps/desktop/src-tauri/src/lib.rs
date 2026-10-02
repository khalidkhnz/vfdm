mod bridge;
mod commands;
mod events;
mod settings;
mod state;

use state::AppState;
use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,vfdm_engine=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let state = tauri::async_runtime::block_on(AppState::init(app.handle()))?;
            app.manage(state.clone());
            events::spawn_forwarder(app.handle().clone(), state.engine.subscribe());
            bridge::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_downloads,
            commands::add_download,
            commands::pause_download,
            commands::resume_download,
            commands::retry_download,
            commands::cancel_download,
            commands::remove_download,
            commands::open_file,
            commands::reveal_file,
            commands::get_settings,
            commands::set_settings,
            commands::get_bridge_info,
            commands::regenerate_token,
            commands::pick_download_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(state.engine.shutdown());
            }
        });
}
