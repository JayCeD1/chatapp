use crate::db_queries::{
    add_room_member, create_room, create_user, get_chat_rooms, get_departments, get_room_messages,
    get_room_reactions, get_rooms_by_department, get_unread_counts, get_user_by_id, get_users,
    join_room, leave_room, list_users, save_message, search_messages, touch_last_read,
    update_user_online_status, upsert_user,
};
use crate::sockets::{
    client_add_member, client_connect_to_server, client_delete_message, client_disconnect,
    client_edit_message, client_join_room, client_leave_room, client_toggle_reaction,
    client_typing, discover_servers, get_server_info, request_history, send_as_client,
    send_as_server_participant, server_add_member, server_delete_message, server_edit_message,
    server_leave_room, server_listen_as_participant, server_participant_disconnect,
    server_participant_join_room, server_toggle_reaction, server_typing, AppState,
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
    // Structured logging. Default level is INFO (matching the prior println behavior);
    // override with e.g. `RUST_LOG=nutler_lib=debug`. try_init so tests/re-entry don't panic.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::new(AppState {
            server_streams: Arc::new(tokio::sync::Mutex::new(Default::default())),
            client_stream: Arc::new(tokio::sync::Mutex::new(None)),
            client_transport: Arc::new(tokio::sync::Mutex::new(None)),
            client_listener: Arc::new(tokio::sync::Mutex::new(None)),
            client_heartbeat: Arc::new(tokio::sync::Mutex::new(None)),
            room_clients: Arc::new(tokio::sync::Mutex::new(Default::default())),
            ip_conn_counts: Arc::new(tokio::sync::Mutex::new(Default::default())),
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

            // The DB holds message history and is not encrypted at rest, so restrict it to
            // the owner on Unix (other local accounts shouldn't be able to read it). The
            // WAL/SHM sidecars carry data too; lock them and the parent dir down as well.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &app_config_dir,
                    std::fs::Permissions::from_mode(0o700),
                );
                for p in [
                    db_path.clone(),
                    db_path.with_extension("db-wal"),
                    db_path.with_extension("db-shm"),
                ] {
                    if p.exists() {
                        let _ =
                            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                    }
                }
            }

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
            create_room,
            add_room_member,
            client_add_member,
            server_add_member,
            list_users,
            join_room,
            leave_room,
            // Message management
            save_message,
            get_room_messages,
            search_messages,
            get_room_reactions,
            get_unread_counts,
            touch_last_read,
            client_toggle_reaction,
            server_toggle_reaction,
            client_typing,
            server_typing,
            request_history,
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
            // Message edit/delete
            client_edit_message,
            server_edit_message,
            client_delete_message,
            server_delete_message,
            // Logout/teardown
            client_disconnect,
            server_participant_disconnect
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application Jesse => ");
}
