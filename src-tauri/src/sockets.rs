use crate::db_queries::save_message_internal;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{Emitter, State};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

//Better indexing and room management
#[derive(Debug, Clone)]
pub struct ClientConnection {
    pub stream: Arc<Mutex<TcpStream>>,
    pub addr: SocketAddr,
    pub username: String,
    pub current_room: String,
    pub room_id: u64,
    pub user_id: u64,
    pub connected_at: std::time::SystemTime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppState {
    #[serde(skip)]
    // Use user_id as key for O(1) lookups
    pub server_streams: Arc<Mutex<HashMap<u64, ClientConnection>>>,
    #[serde(skip)]
    // Separate client stream management
    pub client_stream: Arc<Mutex<Option<TcpStream>>>,
    #[serde(skip)]
    // Track which users are in which rooms for efficient broadcasting
    pub room_clients: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    pub username: String,
    pub user_id: Option<u64>,
    pub is_server: bool,
    pub current_room: String,
    pub current_room_id: Option<u64>,
    pub server_addr: Option<SocketAddr>,
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

#[derive(Serialize, Deserialize, Clone, PartialEq)]
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
pub fn discover_servers(_app: tauri::AppHandle) -> Vec<ServerInfo> {
    let mut servers = Vec::new();
    let base_ip = "192.168.1"; // Common local network range
    let port = 3625;

    // Scan common local network ranges
    for i in 1..=254 {
        let ip = format!("{}.{}", base_ip, i);
        let addr = format!("{}:{}", ip, port);

        match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100)) {
            Ok(_) => {
                servers.push(ServerInfo {
                    address: ip.clone(),
                    port,
                    name: format!("Chat Server at {}", ip),
                    user_count: 0, // Would need to implement server info query
                });
            }
            Err(_) => {}
        }
    }

    // Also try other common local network ranges
    let other_ranges = ["10.0.0", "172.16.0", "192.168.0"];
    for range in other_ranges {
        for i in 1..=50 {
            // Scan fewer IPs for other ranges
            let ip = format!("{}.{}", range, i);
            let addr = format!("{}:{}", ip, port);

            match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(100)) {
                Ok(_) => {
                    servers.push(ServerInfo {
                        address: ip.clone(),
                        port,
                        name: format!("Chat Server at {}", ip),
                        user_count: 0,
                    });
                }
                Err(_) => {}
            }
        }
    }

    servers
}

// Enhanced server that can handle multiple clients
#[tauri::command]
pub fn server_listen(
    app: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    db: State<'_, SqlitePool>,
    username: String,
    user_id: u64,
    port: Option<u16>,
) -> Result<(), String> {
    let port = port.unwrap_or(3625);
    let bind_addr = format!("0.0.0.0:{}", port); // Bind to all interfaces for network access

    let socket = TcpListener::bind(&bind_addr)
        .map_err(|e| format!("Failed to bind to {}: {}", bind_addr, e))?;
    let server_addr = socket.local_addr()
        .map_err(|e| format!("Failed to get server address: {}", e))?;

    println!("Server listening on: {}", server_addr);

    // Update state
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.server_addr = Some(server_addr);
        state_guard.username = username.clone();
        state_guard.user_id = Some(user_id);
        state_guard.is_server = true;
    }

    //  Server connects to itself as client
    let app_clone = app.clone();
    // Clone the Arc<Mutex<AppState>> directly
    let state_clone = Arc::clone(&state.inner());
    let pool_clone = db.inner().clone();
    let username_clone = username.clone();

    thread::spawn(move || {
        // Connect as a client to self first
        if let Err(e) = client_connect_internal(
            app_clone.clone(),
            state_clone.clone(),
            format!("127.0.0.1:{}", port),
            username_clone.clone(),
            user_id,
            "Company Wide".to_string(),
            1
        ) {
            eprintln!("Server self-connect failed: {}", e);
            return;
        }

        // Then start accepting other clients
        for stream in socket.incoming() {
            match stream {
                Ok(stream) => {
                    let app_handle = app_clone.clone();
                    let state_handle = state_clone.clone();
                    let pool_handle = pool_clone.clone();


                    thread::spawn(move || {
                        if let Err(e) = handle_client_connection(app_handle, state_handle, stream, pool_handle) {
                            eprintln!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => eprintln!("Failed to accept connection: {}", e),
            }
        }
    });

    Ok(())
}

fn handle_client_connection(
    app: tauri::AppHandle,
    state: Arc<Mutex<AppState>>,
    mut stream: TcpStream,
    pool: SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    let peer_addr = stream.peer_addr()?;
    println!("New client connection from: {}", peer_addr);

    let mut client_info: Option<ClientConnection> = None;

    loop {
        let mut buffer = [0u8; 4];
        match stream.read_exact(&mut buffer) {
            Ok(()) => {
                let msg_len = u32::from_be_bytes(buffer) as usize;

                if msg_len > 10_000_000 {
                    return Err(format!("Message too large: {} bytes", msg_len).into());
                }

                let mut message_buffer = vec![0u8; msg_len];
                stream.read_exact(&mut message_buffer)?;

                let message_str = std::str::from_utf8(&message_buffer)?;
                let message: Message = serde_json::from_str(message_str)?;

                //Handle client registration
                if message.message_type == MessageType::Connect {
                    client_info = Some(ClientConnection {
                        stream: Arc::new(Mutex::new(stream.try_clone()?)),
                        addr: peer_addr,
                        username: message.username.clone(),
                        current_room: message.room.clone(),
                        room_id: message.room_id,
                        user_id: message.user_id,
                        connected_at: std::time::SystemTime::now(),
                    });

                    //Add to the server's stream list using user_id as a key
                    {
                        let state_guard = state.lock().unwrap();
                        state_guard
                            .server_streams
                            .lock()
                            .unwrap()
                            .insert(message.user_id, client_info.as_ref().unwrap().clone());

                        //Add to room tracking
                        state_guard
                            .room_clients
                            .lock()
                            .unwrap()
                            .entry(message.room.clone())
                            .or_insert_with(Vec::new)
                            .push(message.user_id);
                    }
                    println!(
                        "Client registered: {} (ID: {}) in room {}",
                        message.username, message.user_id, message.room
                    );
                }
                handle_server_message(app.clone(), state.clone(), message, pool.clone())?;
            }
            Err(e) => {
                println!("Connection closed: {} - {}", peer_addr, e);
                break;
            }
        }
    }
    //Clean up with proper error handling
    if let Some(client) = client_info {
        tauri::async_runtime::block_on(async {
            if let Err(e) = clean_client(&state, &app, client, &pool).await {
                eprintln!("Cleanup error: {}", e);
            }

        });
    }

    Ok(())
}


//Separate cleanup function
async fn clean_client(
    state: &Arc<Mutex<AppState>>,
    app: &tauri::AppHandle,
    client: ClientConnection,
    pool: &SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        let state_guard = state.lock().unwrap();

        //Remove from server's stream list
        {
            let mut streams = state_guard.server_streams.lock().unwrap();
            streams.remove(&client.user_id);
        }

        //Remove from room tracking
        {
            let mut rooms = state_guard.room_clients.lock().unwrap();
            if let Some(users) = rooms.get_mut(&client.current_room) {
                users.retain(|&id| id != client.user_id);
            }
        }
    }
    println!("Client disconnected: {} (ID: {})", client.username, client.user_id);

    //Save the disconnect message to the database
    let disconnect_msg = Message {
        message_type: MessageType::Disconnect,
        username: client.username.clone(),
        user_id: client.user_id,
        message: format!("{} left the chat", client.username),
        message_id: Uuid::new_v4().to_string(),
        room: client.current_room.clone(),
        room_id: client.room_id,
        created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        is_emoji: false,
    };

    //Save the disconnect message to the database
    save_message_internal(pool, client.room_id as i64, client.user_id as i64, disconnect_msg.message.clone(), "Disconnect".to_string(), false).await?;

    //Broadcast disconnect (without app.emit to avoid duplicates)
    broadcast_to_room_efficient(app, state, &client.current_room, &disconnect_msg, Some(client.user_id));

    Ok(())
}

//More efficient fir room broadcasting
fn broadcast_to_room_efficient(_app: &tauri::AppHandle, state: &Arc<Mutex<AppState>>, target_room: &str, message: &Message, exclude_user_id: Option<u64>) {
    let state_guard = state.lock().unwrap();
    let streams = state_guard.server_streams.lock().unwrap();
    let room_clients = state_guard.room_clients.lock().unwrap();

    //Only iterate over users in the target room
    if let Some (user_ids) = room_clients.get(target_room){
        for &user_id in user_ids {
            //Skip the excluded user
            if let Some(exclude_user_id) = exclude_user_id {
                if user_id == exclude_user_id {
                    continue;
                }
            }

            if let Some (client_conn) = streams.get(&user_id){
               let mut stream_clone = match client_conn.stream.try_lock() {
                   Ok(guard) => match guard.try_clone() {
                       Ok(stream) => stream,
                       Err(e) => {
                           eprintln!("Failed to clone stream for user {}: {}",user_id, e);
                           continue;
                       },
                   }
                   Err(e) => {
                       eprintln!("Failed tp acquire lock for user {}: {}",user_id, e);
                       continue;
                   }
               };
                if let Err (e) = send_message_with_length(&mut stream_clone, message){
                    eprintln!("Failed to send message to {} ({}): {}", client_conn.username, client_conn.addr, e);
                }
            }
        }
    }
    // REMOVED: app.emit to prevent duplicates in server mode
    // The server-hosting client will receive the message through its client_stream
}

fn handle_server_message(app: tauri::AppHandle, state: Arc<Mutex<AppState>>, message: Message, pool: SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    match message.message_type {
        MessageType::Connect => {
            //Save connect the message to the db
            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err (e) = save_message_internal(&pool_clone, msg_clone.room_id as i64, msg_clone.user_id as i64, msg_clone.message, "Connect".to_string(), false).await {
                    eprintln!("Failed to save connect message to db: {}", e);
                }
            });
            broadcast_to_room_efficient(&app, &state, &message.room, &message, None);
        }
        MessageType::Chat => {
            //save to db
            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err (e) = save_message_internal(&pool_clone, msg_clone.room_id as i64, msg_clone.user_id as i64, msg_clone.message, "Chat".to_string(), false).await {
                    eprintln!("Failed to save chat message to db: {}", e);
                }
            });
            broadcast_to_room_efficient(&app, &state, &message.room, &message, None);
        }
        MessageType::RoomJoin => {
            //Update client's room and room tracking
            {
                let state_guard = state.try_lock().unwrap();

                {
                    let mut server_streams_guard = state_guard.server_streams.try_lock().unwrap();
                    if let Some(client) = server_streams_guard.get_mut(&message.user_id) {
                        //Remove from the old room
                        if let Some(users) = state_guard.room_clients.try_lock().unwrap().get_mut(&client.current_room) {
                            users.retain(|&id| id != message.user_id);
                        }

                        //Update client info
                        client.current_room = message.room.clone();
                        client.room_id = message.room_id;

                        //Add to a new room
                        state_guard.room_clients.try_lock().unwrap()
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
                if let Err (e) = save_message_internal(&pool_clone, msg_clone.room_id as i64, msg_clone.user_id as i64, msg_clone.message, "RoomJoin".to_string(), false).await {
                    eprintln!("Failed to save room join message to db: {}", e);
                }
            });
            broadcast_to_room_efficient(&app, &state, &message.room, &message, None);
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn client_connect(
    app: tauri::AppHandle,
    state: State<Arc<Mutex<AppState>>>,
    host: String,
    username: String,
    user_id: u64,
    room: String,
    room_id: u64
) -> Result<(), String> {
   client_connect_internal(
       app,
       Arc::clone(&state.inner()),
       host,
       username,
       user_id,
       room,
       room_id
   )
}

fn client_connect_internal(
    app: tauri::AppHandle,
    state: Arc<Mutex<AppState>>,
    host: String,
    username: String,
    user_id: u64,
    room: String,
    room_id: u64
)-> Result<(), String> {
    let stream = TcpStream::connect(&host)
        .map_err(|e| format!("Failed to connect to {}: {}", host, e))?;

    // Update state
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.username = username.clone();
        state_guard.user_id = Some(user_id);
        state_guard.current_room = room.clone();
        state_guard.current_room_id = Some(room_id);
        state_guard.is_server = false;
        *state_guard.client_stream.lock().unwrap() = Some(stream.try_clone()
            .map_err(|e| format!("Failed to clone stream: {}", e))?);
    }

    // Send connect message
    let message = Message {
        message_type: MessageType::Connect,
        username: username.clone(),
        user_id,
        message: format!("{} joined the chat", username),
        room: room.clone(),
        room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("Time error: {}", e))?
            .as_secs(),
        is_emoji: false,
        message_id: Uuid::new_v4().to_string(),
    };

    let mut stream_clone = stream.try_clone()
        .map_err(|e| format!("Failed to clone stream: {}", e))?;
    send_message_with_length(&mut stream_clone, &message)
        .map_err(|e| format!("Failed to send connect message: {}", e))?;

    // IMPROVED: Start listener with reconnection capability
    start_client_listener_with_reconnection(app, stream);

    Ok(())
}

async fn client_connect_internal_async(
    app: tauri::AppHandle,
    state: Arc<Mutex<AppState>>,
    host: String,
    username: String,
    user_id: u64,
    room: String,
    room_id: u64
) -> Result<(), String>{

    // Connect using std::net first (blocking)
    let std_stream = TcpStream::connect(&host)
        .map_err(|e| format!("Failed to connect to {}: {}", host, e))?;

    // Set non-blocking for tokio conversion
    std_stream.set_nonblocking(true)
        .map_err(|e| format!("Failed to set non-blocking: {}", e))?;

    // Update state with std stream (your existing structure!)
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.username = username.clone();
        state_guard.user_id = Some(user_id);
        state_guard.current_room = room.clone();
        state_guard.current_room_id = Some(room_id);
        state_guard.is_server = false;

        // Store the std stream clone (works with your existing AppState)
        *state_guard.client_stream.lock().unwrap() = Some(
            std_stream.try_clone()
                .map_err(|e| format!("Failed to clone stream: {}", e))?
        );
    }

    // Create connect message
    let message = Message {
        message_type: MessageType::Connect,
        user_id,
        username: username.clone(),
        message: format!("{} joined the chat", username),
        room: room.clone(),
        room_id,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
        message_id: Uuid::new_v4().to_string(),
    };

    // Convert to tokio stream just for async operations
    let tokio_stream = tokio::net::TcpStream::from_std(std_stream)
        .map_err(|e| format!("Failed to convert to tokio stream: {}", e))?;

    // Split for concurrent read/write
    let (read_half, mut write_half) = tokio_stream.into_split();

    // Send connect message async
    send_message_with_length_async(&mut write_half, &message)
        .await
        .map_err(|e| format!("Failed to send connect message: {}", e))?;

    // Start listener with async read half
    start_client_listener_with_reconnection_async(app, read_half);

    Ok(())
}


// Client listener with reconnection logic
fn start_client_listener_with_reconnection(app: tauri::AppHandle, mut stream: TcpStream) {
   let peer_addr = stream.peer_addr();
    thread::spawn(move || {
        loop {
            let mut len_bytes = [0u8; 4];
            match stream.read_exact(&mut len_bytes) {
                Ok(()) => {
                    let msg_len = u32::from_be_bytes(len_bytes) as usize;
                    //TODO (Is check msg_len > 10_000_000 necessary here as well)

                    let mut message_buffer = vec![0u8; msg_len];
                    match stream.read_exact(&mut message_buffer) {
                        Ok(()) => {
                            if let Ok(message_str) = std::str::from_utf8(&message_buffer) {
                                if let Err(e) = app.emit("message", &message_str) {
                                    eprintln!("Failed to emit message: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Client read error: {}, peer: {:?}", e, peer_addr);
                            break;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Client stream closed: {}, peer: {:?}", e, peer_addr);
                    //Notify the frontend of connection loss
                    if let Err (emit_err) = app.emit("connection_lost", ()){
                        eprintln!("Failed to emit connection lost: {}", emit_err);
                    }
                    break;
                }
            }
        }
    });
}

// Updated listener function
fn start_client_listener_with_reconnection_async(
    app: tauri::AppHandle,
    read_half: tokio::net::tcp::OwnedReadHalf
) {
    tauri::async_runtime::spawn(async move {
        let mut read_stream = read_half;
        let peer_addr = read_stream.peer_addr();
        loop {
        let mut len_bytes = [0u8; 4];
            match read_stream.read_exact(&mut len_bytes).await {
                Ok(0) => {
                    println!("Connection closed by server");
                    break;
                }
                Ok(n) => {
                    // Process received data
                    let msg_len = u32::from_be_bytes(len_bytes) as usize;
                    let mut message_buffer = vec![0u8; msg_len];
                    match read_stream.read_exact(&mut message_buffer).await {
                        Ok(0) => {
                            println!("Failed to read message:");
                            break;
                        }
                        Ok(_n) => {
                            if let Ok(message_str) = std::str::from_utf8(&message_buffer) {
                                if let Err(e) = app.emit("message", &message_str) {
                                    eprintln!("Failed to emit message: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Client read error: {}", e);
                            break;
                        }
                    }

                }
                Err(e) => {
                    // Handle reconnection logic
                    eprintln!("Client stream closed: {}, peer: {:?}", e, peer_addr);
                    //Notify the frontend of connection loss
                    if let Err (emit_err) = app.emit("connection_lost", ()){
                        eprintln!("Failed to emit connection lost: {}", emit_err);
                    }
                    break;
                }
            }
        }
    });
}
//Always use client_stream for consistency even in server mode
#[tauri::command(rename_all = "snake_case")]
pub fn send(
    state: State<'_, Arc<Mutex<AppState>>>,
    message: String,
    user_id: u64,
    room: String,
    room_id: u64,
    is_emoji: bool,
) -> Result<(), String> {
    let state_guard = state.try_lock().unwrap();

    let chat_message = Message {
        message_type: MessageType::Chat,
        username: state_guard.username.clone(),
        user_id,
        message,
        room_id,
        room,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji,
        message_id: Uuid::new_v4().to_string(),
    };

    // Always use client_stream for consistency
    if let Some(ref mut stream) = *state_guard.client_stream.try_lock().unwrap() {
        let mut stream_clone = stream.try_clone()
            .map_err(|e| format!("Failed to clone stream: {}", e))?;
        send_message_with_length(&mut stream_clone, &chat_message)
            .map_err(|e| format!("Failed to send message: {}", e))?;
    }else {
        return Err("Not connected to server".to_string())
    }
    Ok(())
}

#[tauri::command]
pub fn get_server_info(state: State<'_, Arc<Mutex<AppState>>>) -> Option<String> {
    let state_guard = state.lock().unwrap();
    state_guard.server_addr.map(|addr| addr.to_string())
}

fn get_data(app: tauri::AppHandle, mut stream: TcpStream) {
    thread::spawn(move || {
        let mut buffer = [0; 1024];

        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break, // Connection closed
                Ok(n) => {
                    let message_data = &buffer[..n];
                    if let Ok(message_str) = std::str::from_utf8(message_data) {
                        app.emit("message", &message_str).unwrap();
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn get_data_with_length_prefix(app: tauri::AppHandle, mut stream: TcpStream) {
    thread::spawn(move || {
        loop {
            //Read length header
            let mut len_bytes = [0u8; 4];
            match stream.read_exact(&mut len_bytes) {
                Ok(()) => {
                    let msg_len = u32::from_be_bytes(len_bytes) as usize;

                    //Read message payload
                    let mut message_buffer = vec![0u8; msg_len];
                    match stream.read_exact(&mut message_buffer) {
                        Ok(()) => {
                            if let Ok(message_str) = std::str::from_utf8(&message_buffer) {
                                //Emit the message to the frontend
                                app.emit("message", &message_str).unwrap();
                            }
                        }
                        Err(_) => break,
                    }
                }
                Err(_) => break, //Connection closed
            }
        }
    });
}

fn send_message_with_length(
    stream: &mut TcpStream,
    message: &Message,
) -> Result<(), Box<dyn std::error::Error>> {
    // Serialize message to JSON
    let payload = serde_json::to_string(message)?;
    let len = payload.len() as u32;

    // Send length (4 bytes) then payload
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;

    Ok(())
}

// New async version for tokio operations
async fn send_message_with_length_async(
    stream: &mut tokio::net::tcp::OwnedWriteHalf,
    message: &Message,
) -> Result<(), Box<dyn std::error::Error>> {
    let serialized = serde_json::to_string(message)?;
    let length = serialized.len() as u32;

    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(serialized.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}
