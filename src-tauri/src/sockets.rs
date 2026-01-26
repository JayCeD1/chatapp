use crate::db_queries::save_message_internal;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, State};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::timeout;
use uuid::Uuid;

//Better indexing and room management
#[derive(Debug, Clone)]
pub struct ClientConnection {
    // Store only the write half for sending/broadcasting
    pub stream: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    pub addr: SocketAddr,
    pub username: String,
    pub current_room: String,
    pub room_id: u64,
    pub user_id: u64,
    pub connected_at: std::time::SystemTime,
}

#[derive(Debug)]
pub struct AppState {
    // Async collections behind Arcs so AppState can be shared easily
    // Use user_id as key for O(1) lookups
    pub server_streams: Arc<tokio::sync::Mutex<HashMap<u64, ClientConnection>>>,
    // Separate client stream management
    pub client_stream: Arc<tokio::sync::Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>>,
    // Track which users are in which rooms for efficient broadcasting
    pub room_clients: Arc<tokio::sync::Mutex<HashMap<String, Vec<u64>>>>,

    // Use RwLock for frequently-read scalar fields
    pub username: tokio::sync::RwLock<String>,
    pub user_id: tokio::sync::RwLock<Option<u64>>,
    pub is_server: tokio::sync::RwLock<bool>,
    pub current_room: tokio::sync::RwLock<String>,
    pub current_room_id: tokio::sync::RwLock<Option<u64>>,
    pub server_addr: tokio::sync::RwLock<Option<SocketAddr>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub message_type: MessageType,
    pub username: String,
    pub user_id: u64,
    pub message: String,
    pub message_id: String,
    pub room: String,
    pub room_id: u64,
    pub created_at: u64,
    pub is_emoji: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum MessageType {
    Connect,
    Disconnect,
    Chat,
    RoomJoin,
    RoomLeave,
    UserList,
    ServerAck,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RoomInfo {
    pub name: String,
    pub description: String,
    pub user_count: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UserInfo {
    pub username: String,
    pub current_room: String,
    pub is_online: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ServerInfo {
    pub address: String,
    pub port: u16,
    pub name: String,
    pub user_count: usize,
}

// Network discovery - scan for servers on a local network
#[tauri::command]
pub async fn discover_servers(_app: tauri::AppHandle) -> Vec<ServerInfo> {
    let port = 3625;
    let base_ip = "192.168.1"; // Primary range | Common local network range
    let mut targets = Vec::new();
    let other_ranges = ["10.0.0", "172.16.0", "192.168.0"];

    // Scan common local network ranges | Primary /24
    for i in 1..=254 {
        targets.push((format!("{}:{}", base_ip, i), port));
    }
    //Smaller slices of other ranges
    for range in other_ranges {
        for i in 1..=50 {
            targets.push((format!("{}.{}", range, i), port));
        }
    }
    // Limit concurrency to avoid overwhelming the system
    let concurrency = 128;
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();

    for (ip, port) in targets {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        tasks.spawn(async move {
            // Hold the permit for the duration of the probe
            let _p = permit;
            let addr = format!("{}:{}", ip, port);

            // 100ms timeout for probe
            let probe = timeout(Duration::from_millis(100), TcpStream::connect(&addr)).await;
            if let Ok(Ok(_)) = probe {
                Some(ServerInfo {
                    address: ip.clone(),
                    port,
                    name: format!("Chat Server at {}", ip),
                    user_count: 0,
                })
            } else {
                None
            }
        });
    }
    let mut servers = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some(info)) = result {
            servers.push(info);
        }
    }
    servers
}

// MAIN SERVER START FUNCTION - Server as Participant
#[tauri::command]
pub async fn server_listen_as_participant(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    username: String,
    user_id: u64,
    port: Option<u16>,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let port = port.unwrap_or(3625);
    let bind_addr = format!("0.0.0.0:{}", port); // Bind to all interfaces for network access

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("Failed to bind to {}: {}", bind_addr, e))?;
    let server_addr = listener.local_addr()
        .map_err(|e| format!("Failed to get server address: {}", e))?;

    println!("🟢 Server (as participant) listening on: {}", server_addr);

    // Update state - Server is BOTH server AND participant
    {
        *state.server_addr.write().await = Some(server_addr);
        *state.username.write().await = username.clone();
        *state.user_id.write().await = Some(user_id);
        *state.is_server.write().await = true;
        *state.current_room.write().await = room.clone();
        *state.current_room_id.write().await = Some(room_id);

        // Register server as a participant in the room
        let mut rooms = state.room_clients.lock().await;
        rooms.entry(room.clone()).or_default().push(user_id);
    }
    // Send server join message to its own UI immediately
    let join_message = Message {
        message_type: MessageType::Connect,
        username: username.clone(),
        user_id,
        message: format!("🟢 {} started the server and joined the chat", username),
        message_id: Uuid::new_v4().to_string(),
        room: room.clone(),
        room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
    };

    // Save server join to database //Use tauri::async_runtime::spawn for database operations
    let pool_clone = db.inner().clone();
    let msg_clone = join_message.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = save_message_internal(
            &pool_clone,
            room_id as i64, // room_id
            msg_clone.user_id as i64,
            msg_clone.message,
            "Connect".to_string(),
            false,
        )
        .await
        {
            eprintln!("Failed to save server join message: {}", e);
        }
    });

    // Emit join message to server's own UI
    if let Err(e) = app.emit("message", serde_json::to_string(&join_message).unwrap()) {
        eprintln!("Failed to emit server join message: {}", e);
    }

    // Start accepting client connections
    // Use tauri::async_runtime::spawn for the main server loop since it needs app context
    let app_clone = app.clone();
    let state_clone = Arc::clone(&state.inner());
    let pool_clone = db.inner().clone();

    tauri::async_runtime::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("🔵 New client connecting from: {}", addr);
                    let app_handle = app_clone.clone();
                    let state_handle = state_clone.clone();
                    let pool_handle = pool_clone.clone();

                    // Use tokio::spawn for individual client handling (pure network I/O)
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_client_connection(app_handle, state_handle, stream, pool_handle)
                                .await
                        {
                            eprintln!("Failed to handle client connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Failed to accept client connection: {}", e);
                }
            }
        }
    });

    Ok(())
}

// Client handler - uses tokio::spawn internally but can use tauri for DB/events
async fn handle_client_connection(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    stream: TcpStream,
    pool: SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer_addr = stream.peer_addr()?;
    println!("New client connection from: {}", peer_addr);

    let mut client_info: Option<ClientConnection> = None;
    // Split once into owned halves: reader for this loop, writer for broadcasts
    let (mut reader, writer) = stream.into_split();
    let writer_arc = Arc::new(tokio::sync::Mutex::new(writer));
    loop {
        // Step 1: read a 4-byte header (buffer)
        let mut buffer = [0u8; 4];
        match reader.read_exact(&mut buffer).await {
            Ok(_) => {
                let msg_len = u32::from_be_bytes(buffer) as usize;

                // Recoverable: empty message → ignore and wait for next
                if msg_len == 0 {
                    println!("Ignoring empty message");
                    continue;
                }

                if msg_len > 10_000_000 {
                    return Err(format!("Message too large: {} bytes", msg_len).into());
                }

                // Step 2: read the message payload
                let mut message_buffer = vec![0u8; msg_len];
                reader.read_exact(&mut message_buffer).await?;

                let message_str = std::str::from_utf8(&message_buffer)?;
                let message: Message = serde_json::from_str(message_str)?;

                //Handle client registration
                if message.message_type == MessageType::Connect {
                    client_info = Some(ClientConnection {
                        stream: Arc::clone(&writer_arc),
                        addr: peer_addr,
                        username: message.username.clone(),
                        current_room: message.room.clone(),
                        room_id: message.room_id,
                        user_id: message.user_id,
                        connected_at: std::time::SystemTime::now(),
                    });

                    //Add to the server's stream list using user_id as a key
                    {
                        let mut streams = state.server_streams.lock().await;
                        streams.insert(message.user_id, client_info.as_ref().unwrap().clone());

                        //Add to room tracking
                        let mut rooms = state.room_clients.lock().await;
                        rooms
                            .entry(message.room.clone())
                            .or_insert_with(Vec::new)
                            .push(message.user_id);
                    }
                    println!(
                        "Client registered: {} (ID: {}) in room {}",
                        message.username, message.user_id, message.room
                    );
                }
                handle_server_message(app.clone(), state.clone(), message, pool.clone()).await?;
            }

            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                print!("client disconnected: {} - {}", peer_addr, e);
                break;
            }
            Err(e) => {
                eprintln!("Connection closed: {} - {}", peer_addr, e);
                break;
            }
        }
    }
    //Clean up with proper error handling
    if let Some(client) = client_info {
        if let Err(e) = clean_client(&state, &app, client, &pool).await {
            eprintln!("Cleanup error: {}", e);
        }
    }

    Ok(())
}
//Separate cleanup function
async fn clean_client(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    client: ClientConnection,
    pool: &SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        //Remove from server's stream list
        let mut streams = state.server_streams.lock().await;
        streams.remove(&client.user_id);

        //Remove from room tracking
        let mut rooms = state.room_clients.lock().await;
        if let Some(users) = rooms.get_mut(&client.current_room) {
            users.retain(|&id| id != client.user_id);
        }
    }
    println!(
        "Client disconnected: {} (ID: {})",
        client.username, client.user_id
    );

    //Save the disconnect message to the database
    let disconnect_msg = Message {
        message_type: MessageType::Disconnect,
        username: client.username.clone(),
        user_id: client.user_id,
        message: format!("{} left the chat", client.username),
        message_id: Uuid::new_v4().to_string(),
        room: client.current_room.clone(),
        room_id: client.room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        is_emoji: false,
    };

    //Save the disconnect message to the database
    save_message_internal(
        pool,
        client.room_id as i64,
        client.user_id as i64,
        disconnect_msg.message.clone(),
        "Disconnect".to_string(),
        false,
    )
    .await?;

    //Broadcast disconnect
    distribute_message_to_all(
        app,
        state,
        &client.current_room,
        &disconnect_msg,
        Some(client.user_id),
    )
    .await;

    Ok(())
}

// ENHANCED MESSAGE DISTRIBUTION - Handles both network + local UI now async to await tokio locks
async fn distribute_message_to_all(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    target_room: &str,
    message: &Message,
    exclude_user_id: Option<u64>,
) {
    let streams = state.server_streams.lock().await;
    let room_clients = state.room_clients.lock().await;
    let is_server = *state.is_server.read().await;
    let server_user_id = *state.user_id.read().await;

    println!(
        "🔍 Room '{}' contains users: {:?}",
        target_room,
        room_clients.get(target_room)
    );

    //Only iterate over users in the target room i.e., Send to network clients (other machines)
    if let Some(user_ids) = room_clients.get(target_room) {
        println!("📡 Broadcasting to {} network clients", user_ids.len());

        for &user_id in user_ids {
            //Skip the excluded user (usually the sender)
            if let Some(exclude_user_id) = exclude_user_id {
                if user_id == exclude_user_id {
                    continue;
                }
            }

            // Skip server's own user_id for network broadcast
            // (server talks to its UI directly, not via network)
            if is_server && Some(user_id) == server_user_id {
                continue;
            }

            if let Some(client_conn) = streams.get(&user_id) {
                //Spawn an async task to send using the write half.
                let username = client_conn.username.clone();
                let stream_arc = Arc::clone(&client_conn.stream);
                let msg = message.clone();
                tauri::async_runtime::spawn(async move {
                    let mut guard = stream_arc.lock().await;
                    match send_message_with_length(&mut *guard, &msg).await {
                        Ok(_) => println!(" ✅ Sent to {} ({})", username, user_id),
                        Err(e) => println!("   ❌ Failed to send to {}: {}", username, e),
                    }
                });
            }
        }
    }
    // 2. ALWAYS send it to local UI (this machine's interface)
    match app.emit("message", serde_json::to_string(message).unwrap()) {
        Ok(_) => println!("📱 Emitted to local UI successfully"),
        Err(e) => eprintln!("📱 Failed to emit to local UI: {}", e),
    }
}

async fn handle_server_message(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    message: Message,
    pool: SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "🟢 Server handling message: {:?} from {}",
        message.message_type, message.username
    );

    match message.message_type {
        MessageType::Connect => {
            //Save connect the message to the db
            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = save_message_internal(
                    &pool_clone,
                    msg_clone.room_id as i64,
                    msg_clone.user_id as i64,
                    msg_clone.message,
                    "Connect".to_string(),
                    false,
                )
                .await
                {
                    eprintln!("Failed to save connect message to db: {}", e);
                }
            });
            // Distribute to all participants
            distribute_message_to_all(&app, &state, &message.room, &message, None).await;
        }
        MessageType::Chat => {
            //save to db
            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = save_message_internal(
                    &pool_clone,
                    msg_clone.room_id as i64,
                    msg_clone.user_id as i64,
                    msg_clone.message,
                    "Chat".to_string(),
                    false,
                )
                .await
                {
                    eprintln!("Failed to save chat message to db: {}", e);
                }
            });
            // Distribute to all participants (exclude sender to avoid duplicate)
            distribute_message_to_all(&app, &state, &message.room, &message, Some(message.user_id))
                .await;
        }
        MessageType::RoomJoin => {
            //Update client's room and room tracking
            {
                let mut server_streams_guard = state.server_streams.lock().await;
                let mut room_clients_guard = state.room_clients.lock().await;

                {
                    if let Some(client) = server_streams_guard.get_mut(&message.user_id) {
                        //Remove from the old room
                        if let Some(users) = room_clients_guard.get_mut(&client.current_room) {
                            users.retain(|&id| id != message.user_id);
                        }

                        //Update client info
                        client.current_room = message.room.clone();
                        client.room_id = message.room_id;

                        //Add to a new room
                        room_clients_guard
                            .entry(message.room.clone())
                            .or_insert_with(Vec::new)
                            .push(message.user_id);
                    }
                }
            }
            //Save room join to db

            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = save_message_internal(
                    &pool_clone,
                    msg_clone.room_id as i64,
                    msg_clone.user_id as i64,
                    msg_clone.message,
                    "RoomJoin".to_string(),
                    false,
                )
                .await
                {
                    eprintln!("Failed to save room join message to db: {}", e);
                }
            });
            distribute_message_to_all(&app, &state, &message.room, &message, None).await;
        }
        _ => {}
    }
    Ok(())
}

// ENHANCED SEND FUNCTION - Server as Participant
#[tauri::command(rename_all = "snake_case")]
pub async fn send_as_server_participant(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    message: String,
    user_id: u64,
    is_emoji: bool,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let room = state.current_room.read().await.clone();
    let room_id = state.current_room_id.read().await.unwrap_or(1);

    let chat_message = Message {
        message_type: MessageType::Chat,
        username: username.clone(),
        user_id,
        message: message.clone(),
        room_id,
        room,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji,
        message_id: Uuid::new_v4().to_string(),
    };

    // Save to database
    let pool_clone = db.inner().clone();
    let msg_clone = chat_message.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = save_message_internal(
            &pool_clone,
            msg_clone.room_id as i64,
            msg_clone.user_id as i64,
            msg_clone.message,
            "Chat".to_string(),
            false,
        )
        .await
        {
            eprintln!("Failed to save server message to DB: {}", e);
        }
    });

    // Distribute to everyone Send to everyone, no exclusions for server messages
    distribute_message_to_all(&app, state.inner(), &chat_message.room, &chat_message, None).await;

    Ok(())
}

// CLIENT CONNECT FUNCTION - For external clients joining server
#[tauri::command]
pub async fn client_connect_to_server(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    host: String,
    username: String,
    user_id: u64,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    println!("🔵 Client connecting to server at {}", host);

    let stream = TcpStream::connect(&host)
        .await
        .map_err(|e| format!("Failed to connect to {}: {}", host, e))?;

    let (reader, mut writer) = stream.into_split();

    // Update client state
    {
        *state.username.write().await = username.clone();
        *state.user_id.write().await = Some(user_id);
        *state.current_room.write().await = room.clone();
        *state.current_room_id.write().await = Some(room_id);
        *state.is_server.write().await = false;
    };

    // Send connect message BEFORE storing the writer (fully async, no blocking)
    let connect_message = Message {
        message_type: MessageType::Connect,
        username: username.clone(),
        user_id,
        message: format!("🔵 {} joined the chat", username),
        room: room.clone(),
        room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
        message_id: Uuid::new_v4().to_string(),
    };

    send_message_with_length(&mut writer, &connect_message)
        .await
        .map_err(|e| format!("Failed to send connect message to server: {}", e))?;

    // Now store the writer for later sends (await AFTER the std::Mutex is dropped)
    {
        let mut guard = state.client_stream.lock().await;
        *guard = Some(writer);
    }

    start_client_listener(app, reader);

    println!("✅ Client connected successfully");
    Ok(())
}

// CLIENT SEND FUNCTION - For external clients
#[tauri::command(rename_all = "snake_case")]
pub async fn send_as_client(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    message: String,
    user_id: u64,
    is_emoji: bool,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let room = state.current_room.read().await.clone();
    let room_id = state.current_room_id.read().await.unwrap_or(1);

    println!("🔵 Client sending: '{}'", message);

    let chat_message = Message {
        message_type: MessageType::Chat,
        username: username.clone(),
        user_id,
        message: message.clone(),
        room_id,
        room,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji,
        message_id: Uuid::new_v4().to_string(),
    };

    // Send to server via async write half (no blocking, no std::Mutex held)
    {
        let mut guard = state.client_stream.lock().await;
        if let Some(writer) = guard.as_mut() {
            send_message_with_length(writer, &chat_message)
                .await
                .map_err(|e| format!("Failed to send message to server: {}", e))?;
        } else {
            return Err("Not connected to server".to_string());
        }
    }

    // Show in own UI immediately (don't wait for server echo)
    if let Err(e) = app.emit("message", serde_json::to_string(&chat_message).unwrap()) {
        eprintln!("Failed to emit own message to UI: {}", e);
    }

    Ok(())
}

fn start_client_listener(app: tauri::AppHandle, mut reader: tokio::net::tcp::OwnedReadHalf) {
    tauri::async_runtime::spawn(async move {
        println!("🎧 Client listener started");

        loop {
            let mut len_bytes = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut len_bytes).await {
                println!("🔴 Client connection lost: {}", e);
                if let Err(emit_err) = app.emit("connection_lost", ()) {
                    eprintln!("Failed to emit connection lost: {}", emit_err);
                }
                break;
            }
            let msg_len = u32::from_be_bytes(len_bytes) as usize;
            let mut message_buffer = vec![0u8; msg_len];
            match reader.read_exact(&mut message_buffer).await {
                Ok(_n) => {
                    if let Ok(message_str) = std::str::from_utf8(&message_buffer) {
                        println!("🎧 Client received: {}", message_str);
                        if let Err(e) = app.emit("message", message_str) {
                            eprintln!("Failed to emit received message: {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("🔴 Client read error: {}", e);
                    break;
                }
            }
        }
    });
}

#[tauri::command]
pub async fn server_participant_join_room(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    user_id: u64,
    new_room: String,
    new_room_id: u64,
    old_room: String,
) -> Result<(), String> {
    {
        *state.current_room.write().await = new_room.clone();
        *state.current_room_id.write().await = Some(new_room_id);
    }
    let username = state.username.read().await.clone();

    //create room join message
    let room_join_msg = Message {
        message_type: MessageType::RoomJoin,
        username: username.clone(),
        user_id,
        message: format!("📍 {} moved from {} to {}", username, old_room, new_room),
        room: new_room.clone(),
        room_id: new_room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
        message_id: Uuid::new_v4().to_string(),
    };

    {
        //update room tracking manually since server is special case
        let mut room_clients = state.room_clients.lock().await;

        //Remove from old room
        if let Some(users) = room_clients.get_mut(&old_room) {
            users.retain(|&id| id != user_id);
            println!("🔄 Removed server from room '{}'", old_room);
        }

        // Add to new room
        room_clients
            .entry(new_room.clone())
            .or_insert_with(Vec::new)
            .push(user_id);
        println!("🔄 Added server to room '{}'", new_room);

        println!("🔍 Room tracking after switch: {:?}", *room_clients);
    }

    // Save to database
    let pool_clone = db.inner().clone();
    let msg_clone = room_join_msg.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = save_message_internal(
            &pool_clone,
            msg_clone.room_id as i64,
            msg_clone.user_id as i64,
            msg_clone.message,
            "RoomJoin".to_string(),
            false,
        )
        .await
        {
            eprintln!("Failed to save room join: {}", e);
        }
    });

    // Distribute room join message
    distribute_message_to_all(&app, state.inner(), &new_room, &room_join_msg, None).await;

    Ok(())
}

// For client room switching
#[tauri::command]
pub async fn client_join_room(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    new_room: String,
    new_room_id: u64,
) -> Result<(), String> {
    // Capture old state first
    let username = state.username.read().await.clone();
    let old_room = state.current_room.read().await.clone();
    let _old_room_id = state.current_room_id.read().await.unwrap_or(1);
    {
        *state.current_room.write().await = new_room.clone();
        *state.current_room_id.write().await = Some(new_room_id);
    }

    // Send room join to server (server will handle the room tracking update)
    let room_join_msg = Message {
        message_type: MessageType::RoomJoin,
        username: username.clone(),
        user_id,
        message: format!("📍 {} moved from {} to {}", username, old_room, new_room),
        room: new_room.clone(),
        room_id: new_room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
        message_id: Uuid::new_v4().to_string(),
    };

    // Send room join to server via async write half
    {
        let mut guard = state.client_stream.lock().await;
        if let Some(writer) = guard.as_mut() {
            send_message_with_length(writer, &room_join_msg)
                .await
                .map_err(|e| format!("Failed to send room join: {}", e))?;
        } else {
            return Err("Not connected to server".to_string());
        }
    }
    /*-
        - For Connect/RoomJoin/RoomLeave
        The server broadcasts these to everyone (including the sender), and your client
        listener will receive and display them.
        Emitting locally would cause a duplicate
    */
    println!("🔄 Client room switch: {} → {}", old_room, new_room);
    Ok(())
}

#[tauri::command]
pub async fn get_server_info(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    let addr = state.server_addr.read().await.map(|addr| addr.to_string());
    Ok(addr)
}

async fn send_message_with_length(
    stream: &mut tokio::net::tcp::OwnedWriteHalf,
    message: &Message,
) -> Result<(), Box<dyn std::error::Error>> {
    // Serialize message to JSON
    let payload = serde_json::to_string(message)?;
    let len = payload.len() as u32;

    // Send length (4 bytes) then payload
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload.as_bytes()).await?;
    // No explicit flush needed for Tokio TCP; OS buffers will handle it.

    Ok(())
}

#[tauri::command]
pub async fn client_disconnect(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String>{

    //Read current identity from state
    let user_id_opt = {*state.user_id.read().await};
    let username = {state.username.read().await.clone()};
    let room = {state.current_room.read().await.clone()};
    let room_id_opt = {*state.current_room_id.read().await};

    // Best-effort: send a Disconnect to the server before closing
    if let Some(mut write_half) = {
        let mut guard = state.client_stream.lock().await;
        guard.take()
    }{
        // Build a disconnect message (room context if available)
        let disconnect_msg = Message {
            message_type: MessageType::Disconnect,
            username: username.clone(),
            user_id: user_id_opt.unwrap_or(0),
            message: "client disconnect".to_string(),
            message_id: Uuid::new_v4().to_string(),
            room: room.clone(),
            room_id: room_id_opt.unwrap_or(0),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_secs(),
            is_emoji: false,
        };

        // Ignore any send error; we're disconnecting anyway
        let _ = send_message_with_length(&mut write_half, &disconnect_msg).await;
    }

    // Clear local client-mode state
    {
        *state.user_id.write().await = None;
        *state.username.write().await = String::new();
        *state.current_room.write().await = String::new();
        *state.current_room_id.write().await = None;
        *state.server_addr.write().await = None;
        *state.is_server.write().await = false;

    }
    
    //todo confirm if true also check for room leave if necessary || i wonder why not using distribution_message_to_all here instead
    let _ = app.emit("message", ());

    Ok(())
}

#[tauri::command]
pub async fn server_participant_disconnect(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Capture host identity and room
    let host_user_id = { *state.user_id.read().await }.unwrap_or(0);
    let host_username = { state.username.read().await.clone() };
    let host_room = { state.current_room.read().await.clone() };
    let host_room_id = { *state.current_room_id.read().await }.unwrap_or(0);

    // Prepare a disconnect message from the host
    let disconnect_msg = Message {
        message_type: MessageType::Disconnect,
        username: host_username.clone(),
        user_id: host_user_id,
        message: "server_participant_disconnect".to_string(),
        message_id: Uuid::new_v4().to_string(),
        room: host_room.clone(),
        room_id: host_room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        is_emoji: false,
    };

    // Best-effort: broadcast to all connected clients
    {
        let mut guard = state.server_streams.lock().await;
        for (_uid, conn) in guard.iter() {
            // Try to send the disconnect notice to each client
            if let Ok(mut wh) = conn.stream.try_lock() {
                let _ = send_message_with_length(&mut wh, &disconnect_msg).await;
            }
        }
        // After sending, close all connections by dropping their write halves
        guard.clear();
    
    }
    // Clear room->clients index
    {
        let mut rooms = state.room_clients.lock().await;
        rooms.clear();
    }
    // Also clear any client-mode writer if it exists (host may have a client_stream if it connected out)
    {
        let mut client_w = state.client_stream.lock().await;
        client_w.take(); // drop if present
    }
    // Reset identity and server flags
    {
        *state.is_server.write().await = false;
        *state.user_id.write().await = None;
        *state.username.write().await = String::new();
        *state.current_room.write().await = String::new();
        *state.current_room_id.write().await = None;
        *state.server_addr.write().await = None;
    }
    // Optional: notify UI that server hosting stopped
    //todo confirm if true
    let _ = app.emit("message", ());

    Ok(())

}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn read_message_with_length(
        reader: &mut tokio::net::tcp::OwnedReadHalf,
    ) -> Message {
        let mut len_bytes = [0u8; 4];
        reader.read_exact(&mut len_bytes).await.unwrap();
        let msg_len = u32::from_be_bytes(len_bytes) as usize;
        let mut message_buffer = vec![0u8; msg_len];
        reader.read_exact(&mut message_buffer).await.unwrap();
        serde_json::from_slice(&message_buffer).unwrap()
    }

    #[tokio::test]
    async fn send_message_with_length_transmits_message() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, _) = stream.into_split();
            read_message_with_length(&mut reader).await
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let (_reader, mut writer) = stream.into_split();

        let message = Message {
            message_type: MessageType::Chat,
            username: "server".to_string(),
            user_id: 42,
            message: "hello client".to_string(),
            message_id: Uuid::new_v4().to_string(),
            room: "General".to_string(),
            room_id: 1,
            created_at: 1,
            is_emoji: false,
        };

        send_message_with_length(&mut writer, &message)
            .await
            .unwrap();

        let received = server.await.unwrap();
        assert_eq!(received.message, message.message);
        assert_eq!(received.user_id, message.user_id);
        assert_eq!(received.room, message.room);
    }

    #[tokio::test]
    async fn send_message_with_length_supports_multiple_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, _) = stream.into_split();
            let first = read_message_with_length(&mut reader).await;
            let second = read_message_with_length(&mut reader).await;
            (first, second)
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let (_reader, mut writer) = stream.into_split();

        let first = Message {
            message_type: MessageType::Connect,
            username: "client-a".to_string(),
            user_id: 7,
            message: "connected".to_string(),
            message_id: Uuid::new_v4().to_string(),
            room: "Ops".to_string(),
            room_id: 2,
            created_at: 2,
            is_emoji: false,
        };

        let second = Message {
            message_type: MessageType::Chat,
            username: "client-a".to_string(),
            user_id: 7,
            message: "status update".to_string(),
            message_id: Uuid::new_v4().to_string(),
            room: "Ops".to_string(),
            room_id: 2,
            created_at: 3,
            is_emoji: false,
        };

        send_message_with_length(&mut writer, &first).await.unwrap();
        send_message_with_length(&mut writer, &second).await.unwrap();

        let (received_first, received_second) = server.await.unwrap();
        assert_eq!(received_first.message_type, MessageType::Connect);
        assert_eq!(received_second.message_type, MessageType::Chat);
        assert_eq!(received_second.message, second.message);
    }
}
