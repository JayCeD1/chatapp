use std::env;
use std::sync::{Arc, Mutex};
use sqlx::SqlitePool;
use tauri::Manager;
use crate::db_queries::{create_user, get_users, get_user_by_id, update_user_online_status, get_departments, get_chat_rooms, get_rooms_by_department, join_room, leave_room, save_message, get_room_messages, upsert_user};
use crate::sockets::{AppState, server_listen, client_connect, send, get_server_info, discover_servers};

mod migration;
mod db_queries;
mod sockets;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(AppState {
            streams: Arc::new(Mutex::new(std::collections::HashMap::new())),
            username: String::new(),
            current_room: String::new(),
            server_addr: None,
        })))
        .plugin(tauri_plugin_sql::Builder::default()
            .add_migrations("sqlite:nutler.db", migration::get_migrations()).build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // This works in the setup hook where we have access to the app
            let app_data_dir = app.path().app_data_dir()
                .expect("Failed to get app data directory");

            std::fs::create_dir_all(&app_data_dir)
                .expect("Failed to create app data directory");

            let db_path = app_data_dir.join("nutler.db");
            let database_url = format!("sqlite:{}", db_path.to_string_lossy());

            // Connect to database in async runtime
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let pool = SqlitePool::connect(&database_url)
                    .await
                    .expect("Failed to connect to database");

                handle.manage(pool);// <- Add this: makes pool available to commands
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            // User management
            upsert_user, create_user, get_users, get_user_by_id, update_user_online_status,
            // Department management
            get_departments,
            // Chat room management
            get_chat_rooms, get_rooms_by_department, join_room, leave_room,
            // Message management
            save_message, get_room_messages,
            // Socket management
            server_listen, client_connect, send, get_server_info, discover_servers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application Jesse => ");
}
