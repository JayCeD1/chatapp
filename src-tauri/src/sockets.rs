use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::sync::{Mutex, Arc};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use tauri::{Emitter, State};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppState {
    #[serde(skip)]
    pub streams: Arc<Mutex<HashMap<String, TcpStream>>>,
    pub username: String,
    pub current_room: String,
    pub server_addr: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub message_type: MessageType,
    pub username: String,
    pub message: String,
    pub room: String,
    pub timestamp: u64,
    pub is_emoji: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum MessageType {
    Connect,
    Disconnect,
    Chat,
    RoomJoin,
    RoomLeave,
    UserList,
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

// Network discovery - scan for servers on local network
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
        for i in 1..=50 { // Scan fewer IPs for other ranges
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
pub fn server_listen(app: tauri::AppHandle, state: State<'_,Arc<Mutex<AppState>>>, username: String, port: Option<u16>) {
    let port = port.unwrap_or(3625);
    let bind_addr = format!("0.0.0.0:{}", port); // Bind to all interfaces for network access
    
    let socket: TcpListener = TcpListener::bind(&bind_addr).unwrap();
    let server_addr = socket.local_addr().unwrap();
    
    println!("Server listening on: {}", server_addr);
    
    // Update state with server info
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.server_addr = Some(server_addr);
        state_guard.username = username.clone();
    }
    
    // Spawn server listener thread
    let app_clone = app.clone();
    let state_clone = state.inner().clone();
    
    thread::spawn(move || {
        for stream in socket.incoming() {
            match stream {
                Ok(stream) => {
                    let app_handle = app_clone.clone();
                    let state_handle = state_clone.clone();
                    
                    thread::spawn(move || {
                        handle_client_connection(app_handle, state_handle, stream);
                    });
                }
                Err(e) => eprintln!("Failed to accept connection: {}", e),
            }
        }
    });
}

fn handle_client_connection(app: tauri::AppHandle, state: Arc<Mutex<AppState>>, mut stream: TcpStream) {
    let peer_addr = stream.peer_addr().unwrap();
    println!("New connection from: {}", peer_addr);
    
    // Add stream to state
    {
        let state_guard = state.lock().unwrap();
        state_guard.streams.lock().unwrap().insert(peer_addr.to_string(), stream.try_clone().unwrap());
    }
    
    // Handle incoming messages from this client

    let app_clone = app.clone();
    let state_clone = state.clone();
    
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    // Connection closed
                    let state_guard = state_clone.lock().unwrap();
                    state_guard.streams.lock().unwrap().remove(&peer_addr.to_string());
                    break;
                }
                Ok(n) => {
                    let message_data = &buffer[..n];
                    if let Ok(message_str) = std::str::from_utf8(message_data) {
                        if let Ok(message) = serde_json::from_str::<Message>(message_str) {
                            handle_message(app_clone.clone(), Arc::clone(&state_clone), message, peer_addr);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn handle_message(app: tauri::AppHandle, state: Arc<Mutex<AppState>>, message: Message, sender_addr: SocketAddr) {
    match message.message_type {
        MessageType::Connect => {
            // Broadcast user joined to all clients in the room
            broadcast_to_room(&app, &state, &message.room, &message, &sender_addr.to_string());
        }
        MessageType::Chat => {
            // Broadcast chat message to all clients in the room
            broadcast_to_room(&app, &state, &message.room, &message, &sender_addr.to_string());
        }
        MessageType::RoomJoin => {
            // Handle room joining logic
            let mut state_guard = state.lock().unwrap();
            state_guard.current_room = message.room.clone();
            
            // Broadcast room join to all clients
            broadcast_to_room(&app, &state, &message.room, &message, &sender_addr.to_string());
        }
        MessageType::Disconnect => {
            // Remove user from streams and broadcast disconnect
            let state_guard = state.lock().unwrap();
            state_guard.streams.lock().unwrap().remove(&sender_addr.to_string());
            
            broadcast_to_room(&app, &state, &message.room, &message, &sender_addr.to_string());
        }
        _ => {}
    }
}

fn broadcast_to_room(app: &tauri::AppHandle, state: &Arc<Mutex<AppState>>, _room: &str, message: &Message, exclude_addr: &str) {
    let state_guard = state.lock().unwrap();
    let streams = state_guard.streams.lock().unwrap();
    
    let message_json = serde_json::to_string(message).unwrap();
    
    for (addr, stream) in streams.iter() {
        if addr != exclude_addr {
            if let Err(e) = stream.try_clone().unwrap().write_all(message_json.as_bytes()) {
                eprintln!("Failed to send message to {}: {}", addr, e);
            }
        }
    }
    
    // Also emit to frontend
    app.emit("message", &message_json).unwrap();
}

#[tauri::command]
pub fn client_connect(app: tauri::AppHandle, state: State<'_,Arc<Mutex<AppState>>>, host: String, username: String, room: String) {
    let mut stream: TcpStream = TcpStream::connect(&host).unwrap();
    
    let message = Message {
        message_type: MessageType::Connect,
        username: username.clone(),
        message: format!("{} joined the chat", username),
        room: room.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji: false,
    };
    
    let payload = serde_json::to_string(&message).unwrap();
    stream.write_all(payload.as_bytes()).unwrap();
    
    // Update state
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.username = username;
        state_guard.current_room = room;
        state_guard.streams.lock().unwrap().insert("client".to_string(), stream.try_clone().unwrap());
    }
    
    // Start listening for messages
    get_data(app, stream);
}

#[tauri::command(rename_all = "snake_case")]
pub fn send(state: State<'_, Arc<Mutex<AppState>>>, message: String, room: String, is_emoji: bool) {
    let state_guard = state.lock().unwrap();
    let streams = state_guard.streams.lock().unwrap();
    
    let chat_message = Message {
        message_type: MessageType::Chat,
        username: state_guard.username.clone(),
        message,
        room,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        is_emoji,
    };
    
    let payload = serde_json::to_string(&chat_message).unwrap();
    
    // Send to all streams (in a real implementation, you'd filter by room)
    for stream in streams.values() {
        if let Err(e) = stream.try_clone().unwrap().write_all(payload.as_bytes()) {
            eprintln!("Failed to send message: {}", e);
        }
    }
}

#[tauri::command]
pub fn get_server_info(state: State<'_,Arc<Mutex<AppState>>>) -> Option<String> {
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