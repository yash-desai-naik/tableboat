// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod conn;
mod db;
mod paths;
mod settings;
mod state;

use tauri::{Manager, WindowEvent};
use tauri_plugin_window_state::StateFlags;

fn main() {
    tauri::Builder::default()
        // One process for every window: a second launch focuses the running app instead
        // of starting another WebView2 browser process.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.webview_windows().values().next() {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // Restore size and position, but let the UI show the window once it has painted,
        // so there is no white flash on start.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            conn::secrets::init()?;
            let dir = paths::data_dir(app.handle())?;
            app.manage(settings::SettingsStore::load(app.handle())?);
            app.manage(conn::store::ConnectionStore::load(&dir)?);
            app.manage(db::Sessions::default());
            app.manage(db::query::Runs::default());
            app.manage(db::results::ResultStore::default());
            app.manage(ai::ClaudeHub::default());
            Ok(())
        })
        .on_window_event(|window, event| {
            // A window's database sessions (and SSH tunnels) close with it.
            if let WindowEvent::Destroyed = event {
                let app = window.app_handle().clone();
                let label = window.label().to_string();
                app.state::<db::results::ResultStore>().free_window(&label);
                tauri::async_runtime::spawn(async move {
                    ai::stop_window(&app.state::<ai::ClaudeHub>(), &label).await;
                    app.state::<db::Sessions>().close_window(&label).await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::update_settings,
            commands::connections_list,
            commands::connection_save,
            commands::connection_delete,
            commands::connection_duplicate,
            commands::connection_parse_url,
            commands::connection_url,
            commands::connections_export,
            commands::connections_import,
            commands::connection_test,
            commands::sqlite_create_file,
            commands::session_open,
            commands::session_close,
            commands::window_new,
            commands::meta_databases,
            commands::meta_objects,
            commands::meta_search,
            commands::meta_ddl,
            commands::meta_refresh,
            commands::meta_columns,
            commands::query_run,
            commands::query_cancel,
            commands::result_rows,
            commands::result_sort,
            commands::result_filter,
            commands::result_copy,
            commands::result_export,
            commands::result_free,
            commands::table_columns,
            commands::table_preview,
            commands::table_save,
            ai::claude_status,
            ai::claude_send,
            ai::claude_stop,
            ai::claude_new,
            ai::claude_permission_reply,
            ai::claude_sessions,
            ai::claude_session_messages,
            ai::claude_set_context,
            state::state_read,
            state::state_write,
            state::open_settings_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tableboat");
}
