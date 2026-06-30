use crate::db_queries::{
    create_user, get_chat_rooms, get_departments, get_room_messages, get_rooms_by_department,
    get_user_by_id, get_users, join_room, leave_room, save_message, update_user_online_status,
    upsert_user,
};
use crate::sockets::{
    client_connect_to_server, client_disconnect, client_join_room, client_leave_room,
    discover_servers, get_server_info, send_as_client, send_as_server_participant,
    server_leave_room, server_listen_as_participant, server_participant_disconnect,
    server_participant_join_room, AppState,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;

mod db_queries;
mod migration;
mod secure;
mod sockets;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(AppState {
            server_streams: Arc::new(tokio::sync::Mutex::new(Default::default())),
            client_stream: Arc::new(tokio::sync::Mutex::new(None)),
            client_transport: Arc::new(tokio::sync::Mutex::new(None)),
            client_listener: Arc::new(tokio::sync::Mutex::new(None)),
            room_clients: Arc::new(tokio::sync::Mutex::new(Default::default())),
            username: tokio::sync::RwLock::new(String::new()),
            user_id: tokio::sync::RwLock::new(None),
            is_server: tokio::sync::RwLock::new(false),
            current_room: tokio::sync::RwLock::new(String::new()),
            current_room_id: tokio::sync::RwLock::new(None),
            server_addr: tokio::sync::RwLock::new(None),
        }))
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:nutler.db", migration::get_migrations())
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Use the SAME directory tauri-plugin-sql resolves "sqlite:nutler.db" against
            // (app_config_dir). app_data_dir differs from app_config_dir on Linux, which
            // would point migrations and queries at two different files.
            let app_config_dir = app
                .path()
                .app_config_dir()
                .expect("Failed to get app config directory");

            std::fs::create_dir_all(&app_config_dir)
                .expect("Failed to create app config directory");

            let db_path = app_config_dir.join("nutler.db");

            // Build the query pool synchronously so it is managed BEFORE any command can
            // run (avoids a "state not managed" race), with FK enforcement, WAL journaling,
            // and a busy timeout so concurrent writers don't hit "database is locked".
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .busy_timeout(Duration::from_secs(5));

            let pool =
                tauri::async_runtime::block_on(async { SqlitePool::connect_with(options).await })
                    .expect("Failed to connect to database");

            app.manage(pool); // makes the pool available to commands

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // User management
            upsert_user,
            create_user,
            get_users,
            get_user_by_id,
            update_user_online_status,
            // Department management
            get_departments,
            // Chat room management
            get_chat_rooms,
            get_rooms_by_department,
            join_room,
            leave_room,
            // Message management
            save_message,
            get_room_messages,
            // Socket management
            get_server_info,
            discover_servers,
            server_listen_as_participant,
            send_as_server_participant,
            client_connect_to_server,
            send_as_client,
            server_participant_join_room,
            client_join_room,
            client_leave_room,
            server_leave_room,
            // Logout/teardown
            client_disconnect,
            server_participant_disconnect
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application Jesse => ");
}
