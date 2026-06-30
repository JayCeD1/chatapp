use crate::db_queries::{
    add_room_member_internal, delete_message_db, edit_message_db, get_room_messages_internal,
    get_room_reactions_internal, get_unread_counts_internal, list_users_internal,
    room_join_allowed_internal, save_message_internal, toggle_reaction_db,
    touch_last_read_internal, upsert_user_internal,
};
use crate::secure;
use serde::{Deserialize, Serialize};
use snow::TransportState;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
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

/// Maximum size of a single length-prefixed frame payload (10 MiB).
/// Shared by both the server and client read paths so the cap can never drift.
const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;

/// Maximum number of client connections a host will handle concurrently.
const MAX_CONCURRENT_CLIENTS: usize = 256;

/// Maximum length (in characters) of a single chat message.
const MAX_MESSAGE_CHARS: usize = 4000;

/// How often each side sends a zero-length keepalive frame.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// If no frame (including a keepalive) arrives within this window, the peer is
/// treated as dead and the connection is closed. ~3 missed heartbeats.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// Maximum concurrent connections allowed from a single remote IP address.
const MAX_CONN_PER_IP: usize = 16;

/// Spawn a task that sends a zero-length keepalive frame to `writer` every
/// HEARTBEAT_INTERVAL. Zero-length frames are read as `Ok(None)` and skipped before
/// decryption, so they never touch the Noise transport / nonce sequence.
fn spawn_heartbeat(
    writer: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            ticker.tick().await;
            let mut w = writer.lock().await;
            if w.write_all(&0u32.to_be_bytes()).await.is_err() {
                break; // peer gone; the read side will handle cleanup
            }
        }
    })
}

/// Client-side keepalive: same idea, but the client's writer lives behind an Option.
fn spawn_client_heartbeat(
    client_stream: Arc<tokio::sync::Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            ticker.tick().await;
            let mut guard = client_stream.lock().await;
            let stop = match guard.as_mut() {
                Some(w) => w.write_all(&0u32.to_be_bytes()).await.is_err(),
                None => true, // disconnected
            };
            drop(guard);
            if stop {
                break;
            }
        }
    })
}

/// Read one length-prefixed frame: a 4-byte big-endian length header followed by
/// that many payload bytes. Returns `Ok(None)` for a zero-length keep-alive frame.
/// Rejects oversized frames so a malicious peer cannot trigger a huge allocation.
async fn read_frame<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let msg_len = u32::from_be_bytes(len_bytes) as usize;

    if msg_len == 0 {
        return Ok(None);
    }
    if msg_len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Frame too large: {} bytes (max {})",
                msg_len, MAX_FRAME_BYTES
            ),
        ));
    }

    let mut buf = vec![0u8; msg_len];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Current UNIX time in seconds. Returns 0 if the system clock is before the epoch
/// instead of panicking — these run on network-triggered code paths.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Monotonic id assigned to each accepted connection, so a stale connection's
/// teardown can tell whether it still owns the server_streams entry for its user_id
/// (a reconnect may have replaced it).
static NEXT_CONN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A peer's write half + its Noise transport — together, enough to send one
/// encrypted frame. Snapshotted under the streams lock, then used after it drops.
type ClientLink = (
    Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    Arc<tokio::sync::Mutex<TransportState>>,
);

//Better indexing and room management
#[derive(Clone)]
pub struct ClientConnection {
    // Write half + per-connection Noise transport, used together to send/broadcast.
    pub writer: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    pub transport: Arc<tokio::sync::Mutex<TransportState>>,
    pub username: String,
    pub current_room: String,
    pub room_id: u64,
    pub user_id: u64,
    pub conn_id: u64,
}

pub struct AppState {
    // Async collections behind Arcs so AppState can be shared easily
    // Use user_id as key for O(1) lookups
    pub server_streams: Arc<tokio::sync::Mutex<HashMap<u64, ClientConnection>>>,
    // Separate client stream management (write half + matching Noise transport)
    pub client_stream: Arc<tokio::sync::Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>>,
    pub client_transport: Arc<tokio::sync::Mutex<Option<TransportState>>>,
    // Handles to the current client read-listener + heartbeat tasks, so reconnect/
    // disconnect can cancel the stale tasks before starting new ones.
    pub client_listener: Arc<tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    pub client_heartbeat: Arc<tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    // Track which users are in which rooms for efficient broadcasting
    pub room_clients: Arc<tokio::sync::Mutex<HashMap<String, Vec<u64>>>>,
    // Live connection count per remote IP, for the per-IP connection cap.
    pub ip_conn_counts: Arc<tokio::sync::Mutex<HashMap<IpAddr, usize>>>,

    // Use RwLock for frequently-read scalar fields
    pub username: tokio::sync::RwLock<String>,
    pub user_id: tokio::sync::RwLock<Option<u64>>,
    pub is_server: tokio::sync::RwLock<bool>,
    pub current_room: tokio::sync::RwLock<String>,
    pub current_room_id: tokio::sync::RwLock<Option<u64>>,
    pub server_addr: tokio::sync::RwLock<Option<SocketAddr>>,
}

/// Wire-protocol version of the message envelope. Bump when the envelope/payload format
/// changes incompatibly (e.g. plaintext → ciphertext payloads) so old and new can coexist
/// and peers can reject unknown versions. See docs/architecture (ADR-0004).
pub const PROTOCOL_VERSION: u16 = 1;

fn default_protocol_version() -> u16 {
    PROTOCOL_VERSION
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    // Default keeps frames from older/newer peers (or pre-versioning ones) decodable.
    #[serde(default = "default_protocol_version")]
    pub version: u16,
    pub message_type: MessageType,
    pub username: String,
    pub user_id: u64,
    pub message: String,
    pub message_id: String,
    pub room: String,
    pub room_id: u64,
    pub created_at: u64,
    pub is_emoji: bool,
    // Carried only on the Connect frame, so the host can upsert the user into its OWN DB
    // (the identity authority) and assign a globally-unique id. Defaulted/omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum MessageType {
    Connect,
    Disconnect,
    Chat,
    RoomJoin,
    RoomLeave,
    UserList,
    ServerAck,
    Edit,
    Delete,
    Reaction,
    // Host → a single client: a JSON batch of {messages, reactions} for a room, so
    // clients (which keep no local copy of host data) get scrollback on join.
    HistoryResponse,
    // Ephemeral "user is typing" signal, relayed to the room and never persisted.
    // `is_emoji` carries start(true)/stop(false).
    Typing,
    // Client → host: request an older page of a room (`message` carries the before_id
    // cursor as a string). Host → client reply is a HistoryPage to PREPEND.
    HistoryRequest,
    HistoryPage,
    // Host → a single client: JSON [{room_id, count}] of per-room unread counts.
    UnreadCounts,
    // Host → clients: JSON [{id, name, is_online}] directory of users to invite/DM.
    UserDirectory,
    // Client → host: invite a user to a (private) room. `message` carries the target
    // user id; `room_id` the room. The host authorizes by the connection's canonical id.
    AddMember,
    // Host → a single client: their room membership changed; reload the channel list.
    RoomsChanged,
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
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break, // semaphore closed; stop scheduling probes
        };
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
// Tauri command: params map 1:1 to the JS invoke args, so a param struct would
// only add indirection.
#[allow(clippy::too_many_arguments)]
pub async fn server_listen_as_participant(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    username: String,
    user_id: u64,
    port: Option<u16>,
    room: String,
    room_id: u64,
    password: String,
) -> Result<(), String> {
    if password.is_empty() {
        return Err("A room password is required to host".to_string());
    }
    // PSK derived from the room password; every client must present the same password.
    let psk = secure::derive_psk(&password);

    let port = port.unwrap_or(3625);
    let bind_addr = format!("0.0.0.0:{}", port); // Bind to all interfaces for network access

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("Failed to bind to {}: {}", bind_addr, e))?;
    let server_addr = listener
        .local_addr()
        .map_err(|e| format!("Failed to get server address: {}", e))?;

    tracing::info!("🟢 Server (as participant) listening on: {}", server_addr);

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
        version: PROTOCOL_VERSION,
        message_type: MessageType::Connect,
        username: username.clone(),
        user_id,
        message: format!("🟢 {} started the server and joined the chat", username),
        message_id: Uuid::new_v4().to_string(),
        room: room.clone(),
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
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
            msg_clone.message_id,
        )
        .await
        {
            tracing::error!("Failed to save server join message: {}", e);
        }
    });

    // Emit join message to server's own UI
    if let Ok(payload) = serde_json::to_string(&join_message) {
        if let Err(e) = app.emit("message", payload) {
            tracing::error!("Failed to emit server join message: {}", e);
        }
    }

    // Start accepting client connections
    // Use tauri::async_runtime::spawn for the main server loop since it needs app context
    let app_clone = app.clone();
    let state_clone = Arc::clone(state.inner());
    let pool_clone = db.inner().clone();
    // Bound concurrently-handled connections so a flood can't exhaust FDs/memory.
    let conn_limiter = Arc::new(Semaphore::new(MAX_CONCURRENT_CLIENTS));

    tauri::async_runtime::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    // Reject (drop) the connection if we're at capacity rather than
                    // queueing handlers unboundedly.
                    let permit = match Arc::clone(&conn_limiter).try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::error!("⚠️  Connection limit reached; rejecting {}", addr);
                            drop(stream);
                            continue;
                        }
                    };
                    // Per-IP cap: don't let one host monopolize the connection budget.
                    let ip = addr.ip();
                    let over_ip_limit = {
                        let mut counts = state_clone.ip_conn_counts.lock().await;
                        let c = counts.entry(ip).or_insert(0);
                        if *c >= MAX_CONN_PER_IP {
                            true
                        } else {
                            *c += 1;
                            false
                        }
                    };
                    if over_ip_limit {
                        tracing::error!("⚠️  Per-IP connection limit for {}; rejecting", ip);
                        drop(stream);
                        continue;
                    }

                    tracing::info!("🔵 New client connecting from: {}", addr);
                    let app_handle = app_clone.clone();
                    let state_handle = state_clone.clone();
                    let state_dec = state_clone.clone();
                    let pool_handle = pool_clone.clone();

                    // Use tokio::spawn for individual client handling (pure network I/O)
                    tokio::spawn(async move {
                        let _permit = permit; // released when the connection ends
                        if let Err(e) = handle_client_connection(
                            app_handle,
                            state_handle,
                            stream,
                            pool_handle,
                            psk,
                        )
                        .await
                        {
                            tracing::error!("Failed to handle client connection: {}", e);
                        }
                        // Release this IP's slot when the connection ends.
                        let mut counts = state_dec.ip_conn_counts.lock().await;
                        if let Some(c) = counts.get_mut(&ip) {
                            *c = c.saturating_sub(1);
                            if *c == 0 {
                                counts.remove(&ip);
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to accept client connection: {}", e);
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
    psk: [u8; 32],
) -> Result<(), Box<dyn std::error::Error>> {
    let peer_addr = stream.peer_addr()?;
    tracing::info!("New client connection from: {}", peer_addr);

    // Split once into owned halves: reader for this loop, writer for broadcasts.
    let (mut reader, mut writer) = stream.into_split();

    // Authenticate + establish encryption BEFORE trusting anything from this peer.
    // A wrong password fails the handshake and the connection is dropped.
    let transport = match secure::responder_handshake(&mut reader, &mut writer, &psk).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("🔒 Rejected {}: {}", peer_addr, e);
            return Ok(());
        }
    };
    tracing::info!("🔒 Secure session established with {}", peer_addr);

    let writer_arc = Arc::new(tokio::sync::Mutex::new(writer));
    let transport_arc = Arc::new(tokio::sync::Mutex::new(transport));
    let conn_id = NEXT_CONN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Keep the connection alive and let the read-timeout below detect a dead peer.
    let heartbeat = spawn_heartbeat(Arc::clone(&writer_arc));

    let mut client_info: Option<ClientConnection> = None;
    loop {
        // Read one encrypted frame (capped at MAX_FRAME_BYTES), then decrypt it.
        // A timeout means we stopped hearing even keepalives → treat the peer as dead.
        let framed = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut reader)).await;
        match framed {
            Err(_elapsed) => {
                tracing::error!(
                    "⏱️  Read timeout from {} (no heartbeat); closing",
                    peer_addr
                );
                break;
            }
            // Recoverable: empty keep-alive frame → ignore and wait for next
            Ok(Ok(None)) => {
                continue;
            }
            Ok(Ok(Some(ciphertext))) => {
                let plaintext = {
                    let mut ts = transport_arc.lock().await;
                    match secure::decrypt(&mut ts, &ciphertext) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("🔒 Decrypt error from {}: {}", peer_addr, e);
                            break;
                        }
                    }
                };
                let message_str = match std::str::from_utf8(&plaintext) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Invalid UTF-8 from {}: {}", peer_addr, e);
                        break;
                    }
                };
                let message: Message = match serde_json::from_str(message_str) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!("Malformed message from {}: {}", peer_addr, e);
                        break;
                    }
                };

                //Handle client registration
                if message.message_type == MessageType::Connect {
                    // The host is the identity authority: upsert the connecting user into the
                    // host's OWN DB by email and use that globally-unique id, because the id
                    // the client asserts is assigned by its local DB and collides across
                    // machines. No email / a failed upsert means we can't establish a trusted
                    // identity, so we DON'T register — the connection stays unauthenticated and
                    // its later frames are rejected below (closes the asserted-id-hijack hole).
                    // department_id is left None: on a reconnect, room_id is the user's CURRENT
                    // room (not their department), so binding it here would corrupt the record.
                    let canonical = match &message.email {
                        Some(email) => upsert_user_internal(
                            &pool,
                            message.username.clone(),
                            email.clone(),
                            None,
                        )
                        .await
                        .ok()
                        .and_then(|u| u.id),
                        None => None,
                    };
                    if let Some(uid) = canonical.map(|id| id as u64) {
                        let conn = ClientConnection {
                            writer: Arc::clone(&writer_arc),
                            transport: Arc::clone(&transport_arc),
                            username: message.username.clone(),
                            current_room: message.room.clone(),
                            room_id: message.room_id,
                            user_id: uid,
                            conn_id,
                        };
                        client_info = Some(conn.clone());

                        //Add to the server's stream list using the canonical user_id as the key
                        {
                            let mut streams = state.server_streams.lock().await;
                            let mut rooms = state.room_clients.lock().await;

                            // Idempotent (re)registration: if this user is already known
                            // (reconnect / duplicate Connect), drop it from every room first so
                            // membership can't accumulate duplicates that double-deliver.
                            if streams.contains_key(&uid) {
                                for users in rooms.values_mut() {
                                    users.retain(|&id| id != uid);
                                }
                            }

                            streams.insert(uid, conn);

                            //Add to room tracking (deduped)
                            let room_vec =
                                rooms.entry(message.room.clone()).or_insert_with(Vec::new);
                            if !room_vec.contains(&uid) {
                                room_vec.push(uid);
                            }
                        }
                        tracing::info!(
                            "Client registered: {} (id {}) in room {}",
                            message.username,
                            uid,
                            message.room
                        );
                    } else {
                        tracing::warn!(
                            "Rejecting Connect from {} — no resolvable identity (missing/invalid email)",
                            peer_addr
                        );
                    }
                }

                // Reject any frame from a connection that never authenticated (no successful
                // Connect). Without this, a peer could send a first-frame Edit/Delete with a
                // spoofed canonical author id and bypass authorship checks.
                if client_info.is_none() {
                    tracing::warn!(
                        "Ignoring {:?} from unauthenticated connection {}",
                        message.message_type,
                        peer_addr
                    );
                    continue;
                }
                // A single bad message shouldn't kill the connection. Pass the
                // connection's authenticated user_id so edit/delete can't be spoofed.
                let auth_user_id = client_info.as_ref().map(|c| c.user_id);
                if let Err(e) = handle_server_message(
                    app.clone(),
                    state.clone(),
                    message,
                    pool.clone(),
                    auth_user_id,
                )
                .await
                {
                    tracing::error!("Error handling message from {}: {}", peer_addr, e);
                }
            }

            Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                tracing::info!("client disconnected: {} - {}", peer_addr, e);
                break;
            }
            Ok(Err(e)) => {
                tracing::error!("Connection closed: {} - {}", peer_addr, e);
                break;
            }
        }
    }
    heartbeat.abort();

    //Clean up with proper error handling
    if let Some(client) = client_info {
        if let Err(e) = clean_client(&state, &app, client.user_id, conn_id, &pool).await {
            tracing::error!("Cleanup error: {}", e);
        }
    }

    Ok(())
}
//Separate cleanup function
async fn clean_client(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    user_id: u64,
    conn_id: u64,
    pool: &SqlitePool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Remove the LIVE entry ONLY if it's still THIS connection: a reconnect may have
    // replaced server_streams[user_id] with a newer, live connection — tearing that one
    // down would silently drop an actively-connected user. Compare-and-remove by conn_id.
    let removed = {
        let mut streams = state.server_streams.lock().await;
        match streams.get(&user_id) {
            Some(c) if c.conn_id == conn_id => streams.remove(&user_id),
            _ => None, // superseded by a newer connection (or already gone) → no-op
        }
    };
    let Some(client) = removed else {
        return Ok(());
    };
    {
        let mut rooms = state.room_clients.lock().await;
        if let Some(users) = rooms.get_mut(&client.current_room) {
            users.retain(|&id| id != client.user_id);
        }
    }
    tracing::info!(
        "Client disconnected: {} (ID: {})",
        client.username,
        client.user_id
    );

    //Save the disconnect message to the database
    let disconnect_msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Disconnect,
        username: client.username.clone(),
        user_id: client.user_id,
        message: format!("{} left the chat", client.username),
        message_id: Uuid::new_v4().to_string(),
        room: client.current_room.clone(),
        room_id: client.room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };

    //Save the disconnect message to the database
    save_message_internal(
        pool,
        client.room_id as i64,
        client.user_id as i64,
        disconnect_msg.message.clone(),
        "Disconnect".to_string(),
        false,
        disconnect_msg.message_id.clone(),
    )
    .await?;

    //Broadcast disconnect + the updated roster
    distribute_message_to_all(
        app,
        state,
        &client.current_room,
        &disconnect_msg,
        Some(client.user_id),
    )
    .await;
    broadcast_user_list(app, state, &client.current_room).await;

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
    // Briefly hold the collection locks to snapshot the target writers, then release ALL
    // locks before any network I/O or emit (avoids holding mutexes across .await fan-out).
    type Target = (
        Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
        Arc<tokio::sync::Mutex<TransportState>>,
        String,
        u64,
    );
    let targets: Vec<Target> = {
        let streams = state.server_streams.lock().await;
        let room_clients = state.room_clients.lock().await;
        let mut v = Vec::new();
        if let Some(user_ids) = room_clients.get(target_room) {
            for &user_id in user_ids {
                //Skip the excluded user (usually the sender)
                if Some(user_id) == exclude_user_id {
                    continue;
                }
                // NOTE: we intentionally do NOT skip "the server's own user_id" here. The
                // host is never in server_streams, so a host-id lookup below already returns
                // None (no network send). But user_ids are assigned by each instance's local
                // DB and so collide across machines — a client can legitimately share the
                // host's id, and `streams.get` then resolves to that real client. Skipping by
                // id dropped such a client from delivery (they could send but never receive).
                if let Some(conn) = streams.get(&user_id) {
                    v.push((
                        Arc::clone(&conn.writer),
                        Arc::clone(&conn.transport),
                        conn.username.clone(),
                        user_id,
                    ));
                }
            }
        }
        v
    }; // locks released here

    tracing::info!("📡 Broadcasting to {} network clients", targets.len());
    for (writer, transport, username, user_id) in targets {
        let msg = message.clone();
        tauri::async_runtime::spawn(async move {
            match send_secure(&writer, &transport, &msg).await {
                Ok(_) => tracing::info!(" ✅ Sent to {} ({})", username, user_id),
                Err(e) => tracing::info!("   ❌ Failed to send to {}: {}", username, e),
            }
        });
    }
    // 2. ALWAYS send it to local UI (this machine's interface)
    match serde_json::to_string(message) {
        Ok(payload) => match app.emit("message", payload) {
            Ok(_) => tracing::info!("📱 Emitted to local UI successfully"),
            Err(e) => tracing::error!("📱 Failed to emit to local UI: {}", e),
        },
        Err(e) => tracing::error!("📱 Failed to serialize message for local UI: {}", e),
    }
}

/// Build the list of usernames currently present in `room` (server truth, from the
/// host's room_clients). Includes the host itself when it participates in the room.
async fn room_member_names(state: &Arc<AppState>, room: &str) -> Vec<String> {
    let host_id = *state.user_id.read().await;
    let host_name = state.username.read().await.clone();
    // Consistent lock order: server_streams before room_clients.
    let streams = state.server_streams.lock().await;
    let room_clients = state.room_clients.lock().await;
    let mut names = Vec::new();
    if let Some(ids) = room_clients.get(room) {
        for &uid in ids {
            if let Some(conn) = streams.get(&uid) {
                names.push(conn.username.clone());
            } else if Some(uid) == host_id && !host_name.is_empty() {
                names.push(host_name.clone());
            }
        }
    }
    names
}

/// Broadcast the live roster of `room` to everyone in it (and the host's own UI) as a
/// UserList message, so member panels reflect server truth on every membership change.
async fn broadcast_user_list(app: &tauri::AppHandle, state: &Arc<AppState>, room: &str) {
    let names = room_member_names(state, room).await;
    let payload = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
    let msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::UserList,
        username: String::new(),
        user_id: 0,
        message: payload,
        message_id: Uuid::new_v4().to_string(),
        room: room.to_string(),
        room_id: 0,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    distribute_message_to_all(app, state, room, &msg, None).await;
}

/// Send one client the recent history (messages + reactions) of a room from the host's
/// DB. Clients keep no local copy, so this is their scrollback on join. The batch is
/// trimmed to fit a single Noise frame.
async fn send_room_history(
    state: &Arc<AppState>,
    pool: &SqlitePool,
    user_id: u64,
    room: &str,
    room_id: u64,
    before_id: Option<i64>,
    msg_type: MessageType,
) {
    let conn = {
        let streams = state.server_streams.lock().await;
        streams
            .get(&user_id)
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
    };
    let Some((writer, transport)) = conn else {
        return;
    };

    let mut messages = get_room_messages_internal(pool, room_id as i64, 50, before_id)
        .await
        .unwrap_or_default();
    let all_reactions = get_room_reactions_internal(pool, room_id as i64, user_id as i64)
        .await
        .unwrap_or_default();

    // Build the batch for a given message set, carrying ONLY the reactions whose target
    // survives the trim (reactions are room-wide, so an untrimmed list could overflow the
    // frame on its own).
    let make = |msgs: &[crate::db_queries::Message]| -> Message {
        let ids: std::collections::HashSet<&str> = msgs
            .iter()
            .filter_map(|m| m.message_id.as_deref())
            .collect();
        let reactions: Vec<_> = all_reactions
            .iter()
            .filter(|r| ids.contains(r.message_id.as_str()))
            .collect();
        let payload = serde_json::json!({ "messages": msgs, "reactions": reactions }).to_string();
        Message {
            version: PROTOCOL_VERSION,
            message_type: msg_type,
            username: String::new(),
            user_id: 0,
            message: payload,
            message_id: Uuid::new_v4().to_string(),
            room: room.to_string(),
            room_id,
            created_at: now_secs(),
            is_emoji: false,
            email: None,
        }
    };

    // Drop oldest messages (and their reactions) until the SERIALIZED OUTER Message —
    // what send_secure actually encrypts (the inner payload gets JSON-escaped again) —
    // fits one Noise frame with headroom for the AEAD tag.
    let resp = loop {
        let m = make(&messages);
        let outer = serde_json::to_string(&m)
            .map(|s| s.len())
            .unwrap_or(usize::MAX);
        if outer + 64 <= 60_000 || messages.len() <= 1 {
            break m;
        }
        messages.remove(0);
    };

    // If even the trimmed frame can't go out (e.g. one oversized message), send an
    // empty batch so the client clears its loading state instead of spinning forever.
    if send_secure(&writer, &transport, &resp).await.is_err() {
        let empty = make(&[]);
        let _ = send_secure(&writer, &transport, &empty).await;
    }
}

/// Compute a client's per-room unread counts from the host DB and push them as an
/// UnreadCounts frame to just that client. No-op if the client isn't connected.
async fn push_unread(state: &Arc<AppState>, pool: &SqlitePool, user_id: u64) {
    let conn = {
        let streams = state.server_streams.lock().await;
        streams
            .get(&user_id)
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
    };
    let Some((writer, transport)) = conn else {
        return;
    };
    let counts = get_unread_counts_internal(pool, user_id as i64)
        .await
        .unwrap_or_default();
    let payload = serde_json::to_string(&counts).unwrap_or_else(|_| "[]".to_string());
    let msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::UnreadCounts,
        username: String::new(),
        user_id: 0,
        message: payload,
        message_id: Uuid::new_v4().to_string(),
        room: String::new(),
        room_id: 0,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    let _ = send_secure(&writer, &transport, &msg).await;
}

/// Push the user directory (everyone in the host DB) to every connected client + the host's
/// own UI, so invite/DM pickers have someone to choose. Cheap; called when the roster changes.
async fn push_user_directory(app: &tauri::AppHandle, state: &Arc<AppState>, pool: &SqlitePool) {
    let users = list_users_internal(pool).await.unwrap_or_default();
    let payload = serde_json::to_string(&users).unwrap_or_else(|_| "[]".to_string());
    let msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::UserDirectory,
        username: String::new(),
        user_id: 0,
        message: payload,
        message_id: Uuid::new_v4().to_string(),
        room: String::new(),
        room_id: 0,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    let conns: Vec<_> = {
        let streams = state.server_streams.lock().await;
        streams
            .values()
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
            .collect()
    };
    for (writer, transport) in conns {
        let _ = send_secure(&writer, &transport, &msg).await;
    }
    if let Ok(s) = serde_json::to_string(&msg) {
        let _ = app.emit("message", s);
    }
}

/// Tell one user (if connected) that their room membership changed, so their client reloads
/// the channel list (e.g. after being invited to a private channel or DM).
async fn send_rooms_changed(state: &Arc<AppState>, user_id: u64) {
    let conn = {
        let streams = state.server_streams.lock().await;
        streams
            .get(&user_id)
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
    };
    if let Some((writer, transport)) = conn {
        let msg = Message {
            version: PROTOCOL_VERSION,
            message_type: MessageType::RoomsChanged,
            username: String::new(),
            user_id: 0,
            message: String::new(),
            message_id: Uuid::new_v4().to_string(),
            room: String::new(),
            room_id: 0,
            created_at: now_secs(),
            is_emoji: false,
            email: None,
        };
        let _ = send_secure(&writer, &transport, &msg).await;
    }
}

/// After a chat lands in `room`, keep every connected client's unread badge honest:
/// clients currently viewing the room have it marked read (they see it live); clients
/// elsewhere get a fresh unread push (their badge for this room may have grown). This is
/// what makes background-room badges work despite the host only relaying the active room.
async fn notify_unread_for_room(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &SqlitePool,
    room: &str,
    room_id: u64,
) {
    let clients: Vec<(u64, String)> = {
        let streams = state.server_streams.lock().await;
        streams
            .values()
            .map(|c| (c.user_id, c.current_room.clone()))
            .collect()
    };
    for (uid, current_room) in clients {
        if current_room == room {
            let _ = touch_last_read_internal(pool, uid as i64, room_id as i64).await;
        } else {
            push_unread(state, pool, uid).await;
        }
    }

    // The host participant isn't in server_streams, so refresh its own badges here:
    // mark its current room read, then emit authoritative (post-save) counts to its UI.
    if *state.is_server.read().await {
        if let Some(host_id) = *state.user_id.read().await {
            if *state.current_room.read().await == room {
                let _ = touch_last_read_internal(pool, host_id as i64, room_id as i64).await;
            }
            let counts = get_unread_counts_internal(pool, host_id as i64)
                .await
                .unwrap_or_default();
            if let Ok(payload) = serde_json::to_string(&counts) {
                let msg = Message {
                    version: PROTOCOL_VERSION,
                    message_type: MessageType::UnreadCounts,
                    username: String::new(),
                    user_id: 0,
                    message: payload,
                    message_id: Uuid::new_v4().to_string(),
                    room: String::new(),
                    room_id: 0,
                    created_at: now_secs(),
                    is_emoji: false,
                    email: None,
                };
                if let Ok(s) = serde_json::to_string(&msg) {
                    let _ = app.emit("message", s);
                }
            }
        }
    }
}

async fn handle_server_message(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    message: Message,
    pool: SqlitePool,
    // The canonical user_id bound to THIS connection at Connect; used for authorship
    // checks so a peer can't act as another user by spoofing message.user_id.
    auth_user_id: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Re-stamp the frame with the connection's canonical id (the client asserts a
    // per-instance id that collides across machines, and the host's DB is the authority).
    // After this, persistence/delivery/exclusion by message.user_id are all canonical.
    let mut message = message;
    if let Some(uid) = auth_user_id {
        message.user_id = uid;
    }
    // The email was already consumed during Connect registration (above, in the read loop);
    // drop it so it's never relayed to other clients in the distributed Connect notice.
    message.email = None;

    tracing::info!(
        "🟢 Server handling message: {:?} from {}",
        message.message_type,
        message.username
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
                    msg_clone.message_id,
                )
                .await
                {
                    tracing::error!("Failed to save connect message to db: {}", e);
                }
            });
            // Distribute to all participants
            distribute_message_to_all(&app, &state, &message.room, &message, None).await;
            broadcast_user_list(&app, &state, &message.room).await;
            // Seed the freshly-connected client's unread badges.
            if let Some(requester) = auth_user_id {
                push_unread(&state, &pool, requester).await;
            }
            // The roster grew → refresh everyone's invite/DM directory.
            push_user_directory(&app, &state, &pool).await;
        }
        MessageType::Chat => {
            // Distribute first (live delivery to in-room clients), then persist and refresh
            // unread badges in a single task so the unread recompute sees the saved row.
            distribute_message_to_all(&app, &state, &message.room, &message, Some(message.user_id))
                .await;

            let pool_clone = pool.clone();
            let state_clone = Arc::clone(&state);
            let app_clone = app.clone();
            let room = message.room.clone();
            let room_id = message.room_id;
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = save_message_internal(
                    &pool_clone,
                    msg_clone.room_id as i64,
                    msg_clone.user_id as i64,
                    msg_clone.message,
                    "Chat".to_string(),
                    msg_clone.is_emoji,
                    msg_clone.message_id,
                )
                .await
                {
                    tracing::error!("Failed to save chat message to db: {}", e);
                    return;
                }
                notify_unread_for_room(&app_clone, &state_clone, &pool_clone, &room, room_id).await;
            });
        }
        MessageType::RoomJoin => {
            // Use the connection-bound id, never the (spoofable) one in the frame, so a
            // peer can't relocate another user's room or redirect their delivery.
            let actor = auth_user_id.unwrap_or(message.user_id);
            // Enforce private-channel access: a non-member can't join (or pull history for)
            // a private room, even by sending a raw RoomJoin frame.
            if !room_join_allowed_internal(&pool, actor as i64, message.room_id as i64)
                .await
                .unwrap_or(false)
            {
                tracing::warn!(
                    "Denied join to private room {} for user {}",
                    message.room_id,
                    actor
                );
                return Ok(());
            }
            //Update client's room and room tracking
            let mut old_room: Option<String> = None;
            {
                let mut server_streams_guard = state.server_streams.lock().await;
                let mut room_clients_guard = state.room_clients.lock().await;

                if let Some(client) = server_streams_guard.get_mut(&actor) {
                    //Remember + remove from the old room
                    old_room = Some(client.current_room.clone());
                    if let Some(users) = room_clients_guard.get_mut(&client.current_room) {
                        users.retain(|&id| id != actor);
                    }

                    //Update client info
                    client.current_room = message.room.clone();
                    client.room_id = message.room_id;

                    //Add to a new room (deduped)
                    let room_vec = room_clients_guard
                        .entry(message.room.clone())
                        .or_insert_with(Vec::new);
                    if !room_vec.contains(&actor) {
                        room_vec.push(actor);
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
                    msg_clone.message_id,
                )
                .await
                {
                    tracing::error!("Failed to save room join message to db: {}", e);
                }
            });
            distribute_message_to_all(&app, &state, &message.room, &message, None).await;
            broadcast_user_list(&app, &state, &message.room).await;
            if let Some(old) = old_room {
                if old != message.room {
                    broadcast_user_list(&app, &state, &old).await;
                }
            }
            // Give the joining client scrollback for the room they just opened.
            // Only for authenticated connections — never push to a spoofed user_id.
            if let Some(requester) = auth_user_id {
                send_room_history(
                    &state,
                    &pool,
                    requester,
                    &message.room,
                    message.room_id,
                    None,
                    MessageType::HistoryResponse,
                )
                .await;
                // Opening a room marks it read; refresh the client's badges.
                let _ =
                    touch_last_read_internal(&pool, requester as i64, message.room_id as i64).await;
                push_unread(&state, &pool, requester).await;
            }
        }
        MessageType::RoomLeave => {
            // Remove the user from the room they are leaving so the host stops relaying
            // that room to them, then tell the remaining members.
            {
                let mut streams = state.server_streams.lock().await;
                let mut rooms = state.room_clients.lock().await;
                // Clear the connection's current_room (now in the lobby) so a later
                // disconnect doesn't broadcast a stale "left" into the room they left.
                if let Some(conn) = streams.get_mut(&message.user_id) {
                    conn.current_room = String::new();
                }
                if let Some(users) = rooms.get_mut(&message.room) {
                    users.retain(|&id| id != message.user_id);
                }
            }
            let pool_clone = pool.clone();
            let msg_clone = message.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = save_message_internal(
                    &pool_clone,
                    msg_clone.room_id as i64,
                    msg_clone.user_id as i64,
                    msg_clone.message,
                    "RoomLeave".to_string(),
                    false,
                    msg_clone.message_id,
                )
                .await
                {
                    tracing::error!("Failed to save room leave message to db: {}", e);
                }
            });
            distribute_message_to_all(&app, &state, &message.room, &message, Some(message.user_id))
                .await;
            broadcast_user_list(&app, &state, &message.room).await;
        }
        // Edit/Delete events use `message_id` as the TARGET message id. Authorize with
        // the connection's bound user_id (NOT the client-supplied message.user_id), so a
        // peer can only modify messages they actually authored.
        MessageType::Edit => {
            let editor = auth_user_id.unwrap_or(message.user_id) as i64;
            if let Ok(rows) =
                edit_message_db(&pool, &message.message_id, &message.message, editor).await
            {
                if rows > 0 {
                    distribute_message_to_all(&app, &state, &message.room, &message, None).await;
                }
            }
        }
        MessageType::Delete => {
            let editor = auth_user_id.unwrap_or(message.user_id) as i64;
            if let Ok(rows) = delete_message_db(&pool, &message.message_id, editor).await {
                if rows > 0 {
                    let mut del = message.clone();
                    del.message = String::new();
                    distribute_message_to_all(&app, &state, &message.room, &del, None).await;
                }
            }
        }
        // Reaction: message_id = TARGET, message = emoji, reactor = bound user_id. The
        // host toggles in the DB and broadcasts the result with is_emoji = added.
        MessageType::Reaction => {
            let reactor = auth_user_id.unwrap_or(message.user_id);
            if let Ok(added) =
                toggle_reaction_db(&pool, &message.message_id, reactor as i64, &message.message)
                    .await
            {
                let mut evt = message.clone();
                evt.user_id = reactor;
                evt.is_emoji = added; // carries the added(true)/removed(false) result
                distribute_message_to_all(&app, &state, &message.room, &evt, None).await;
            }
        }
        // Typing is ephemeral: relay to the rest of the room (never persisted), keyed
        // off the bound user_id so the sender is correctly excluded.
        MessageType::Typing => {
            let actor = auth_user_id.unwrap_or(message.user_id);
            distribute_message_to_all(&app, &state, &message.room, &message, Some(actor)).await;
        }
        // Client wants an older page of a room; `message` carries the before_id cursor.
        // Reply (only to the authenticated requester) with a HistoryPage to prepend.
        MessageType::HistoryRequest => {
            if let Some(requester) = auth_user_id {
                // Same private-channel gate as RoomJoin — this is an independent path into
                // send_room_history, so without it a non-member could page a private room's
                // history with crafted frames.
                if !room_join_allowed_internal(&pool, requester as i64, message.room_id as i64)
                    .await
                    .unwrap_or(false)
                {
                    tracing::warn!(
                        "Denied history for private room {} to user {}",
                        message.room_id,
                        requester
                    );
                    return Ok(());
                }
                let before_id = message.message.parse::<i64>().ok();
                send_room_history(
                    &state,
                    &pool,
                    requester,
                    &message.room,
                    message.room_id,
                    before_id,
                    MessageType::HistoryPage,
                )
                .await;
            }
        }
        // Client invites a user (`message` = target user id) to a room. Authorized by the
        // connection's canonical id; the invited user (if online) is told to reload rooms.
        MessageType::AddMember => {
            if let Some(actor) = auth_user_id {
                if let Ok(target) = message.message.parse::<i64>() {
                    if add_room_member_internal(&pool, message.room_id as i64, target, actor as i64)
                        .await
                        .is_ok()
                    {
                        send_rooms_changed(&state, target as u64).await;
                    }
                }
            }
        }
        // Disconnect is handled by the connection's EOF cleanup path (clean_client).
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
    if message.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!("Message exceeds {} characters", MAX_MESSAGE_CHARS));
    }

    let username = state.username.read().await.clone();
    let room = state.current_room.read().await.clone();
    let room_id = state.current_room_id.read().await.unwrap_or(1);

    let chat_message = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Chat,
        username: username.clone(),
        user_id,
        message: message.clone(),
        room_id,
        room,
        created_at: now_secs(),
        is_emoji,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };

    // Save to database
    // Distribute to everyone, no exclusions for server messages
    distribute_message_to_all(&app, state.inner(), &chat_message.room, &chat_message, None).await;

    let pool_clone = db.inner().clone();
    let state_clone = Arc::clone(state.inner());
    let app_clone = app.clone();
    let room = chat_message.room.clone();
    let room_id = chat_message.room_id;
    let msg_clone = chat_message.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = save_message_internal(
            &pool_clone,
            msg_clone.room_id as i64,
            msg_clone.user_id as i64,
            msg_clone.message,
            "Chat".to_string(),
            msg_clone.is_emoji,
            msg_clone.message_id,
        )
        .await
        {
            tracing::error!("Failed to save server message to DB: {}", e);
            return;
        }
        notify_unread_for_room(&app_clone, &state_clone, &pool_clone, &room, room_id).await;
    });

    Ok(())
}

// CLIENT CONNECT FUNCTION - For external clients joining server
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command: params map 1:1 to JS invoke args.
pub async fn client_connect_to_server(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    host: String,
    username: String,
    user_id: u64,
    email: String,
    room: String,
    room_id: u64,
    password: String,
) -> Result<(), String> {
    tracing::info!("🔵 Client connecting to server at {}", host);

    let stream = TcpStream::connect(&host)
        .await
        .map_err(|e| format!("Failed to connect to {}: {}", host, e))?;

    let (mut reader, mut writer) = stream.into_split();

    // Authenticate + establish encryption. A wrong password fails the handshake here.
    let psk = secure::derive_psk(&password);
    let transport = secure::initiator_handshake(&mut reader, &mut writer, &psk)
        .await
        .map_err(|e| format!("Secure handshake failed (wrong password?): {}", e))?;
    tracing::info!("🔒 Secure session established with {}", host);

    // Update client state
    {
        *state.username.write().await = username.clone();
        *state.user_id.write().await = Some(user_id);
        *state.current_room.write().await = room.clone();
        *state.current_room_id.write().await = Some(room_id);
        *state.is_server.write().await = false;
    };

    // Store the writer + transport for later (encrypted) sends.
    {
        let mut guard = state.client_stream.lock().await;
        *guard = Some(writer);
    }
    {
        let mut guard = state.client_transport.lock().await;
        *guard = Some(transport);
    }

    // Send the (encrypted) connect message now that the transport is stored.
    let connect_message = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Connect,
        username: username.clone(),
        user_id,
        message: format!("🔵 {} joined the chat", username),
        room: room.clone(),
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: Some(email.clone()),
        message_id: Uuid::new_v4().to_string(),
    };
    send_secure_client(state.inner(), &connect_message)
        .await
        .map_err(|e| format!("Failed to send connect message to server: {}", e))?;

    // Cancel any previous listener + heartbeat (e.g. from a dropped connection) BEFORE
    // starting new ones, so a stale task can't emit a spurious connection_lost and
    // trigger an unwanted reconnect loop.
    {
        let mut guard = state.client_listener.lock().await;
        if let Some(old) = guard.take() {
            old.abort();
        }
    }
    {
        let mut guard = state.client_heartbeat.lock().await;
        if let Some(old) = guard.take() {
            old.abort();
        }
    }
    let listener = start_client_listener(app, reader, Arc::clone(&state.client_transport));
    *state.client_listener.lock().await = Some(listener);
    let heartbeat = spawn_client_heartbeat(Arc::clone(&state.client_stream));
    *state.client_heartbeat.lock().await = Some(heartbeat);

    tracing::info!("✅ Client connected successfully");
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
    if message.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!("Message exceeds {} characters", MAX_MESSAGE_CHARS));
    }

    let username = state.username.read().await.clone();
    let room = state.current_room.read().await.clone();
    let room_id = state.current_room_id.read().await.unwrap_or(1);

    tracing::info!("🔵 Client sending: '{}'", message);

    let chat_message = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Chat,
        username: username.clone(),
        user_id,
        message: message.clone(),
        room_id,
        room,
        created_at: now_secs(),
        is_emoji,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };

    // Send to server over the encrypted channel.
    send_secure_client(state.inner(), &chat_message)
        .await
        .map_err(|e| format!("Failed to send message to server: {}", e))?;

    // Show in own UI immediately (don't wait for server echo)
    if let Ok(payload) = serde_json::to_string(&chat_message) {
        if let Err(e) = app.emit("message", payload) {
            tracing::error!("Failed to emit own message to UI: {}", e);
        }
    }

    Ok(())
}

fn start_client_listener(
    app: tauri::AppHandle,
    mut reader: tokio::net::tcp::OwnedReadHalf,
    transport: Arc<tokio::sync::Mutex<Option<TransportState>>>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        tracing::info!("🎧 Client listener started");

        loop {
            // Same capped framing as the server path; each frame is then decrypted with
            // the shared client transport. A read-timeout (no frame, not even a keepalive)
            // means the host is gone → surface connection_lost so reconnect can kick in.
            let framed = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut reader)).await;
            let ciphertext = match framed {
                Err(_elapsed) => {
                    tracing::info!("⏱️  Client read timeout (host gone)");
                    let _ = app.emit("connection_lost", ());
                    break;
                }
                Ok(Ok(None)) => continue, // empty keep-alive frame
                Ok(Ok(Some(ct))) => ct,
                Ok(Err(e)) => {
                    tracing::info!("🔴 Client connection lost: {}", e);
                    if let Err(emit_err) = app.emit("connection_lost", ()) {
                        tracing::error!("Failed to emit connection lost: {}", emit_err);
                    }
                    break;
                }
            };

            let plaintext = {
                let mut guard = transport.lock().await;
                match guard.as_mut() {
                    Some(ts) => match secure::decrypt(ts, &ciphertext) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("🔒 Client decrypt error: {}", e);
                            break;
                        }
                    },
                    None => {
                        tracing::error!("🔒 No client transport; dropping frame");
                        break;
                    }
                }
            };
            match String::from_utf8(plaintext) {
                Ok(message_str) => {
                    tracing::info!("🎧 Client received: {}", message_str);
                    if let Err(e) = app.emit("message", message_str) {
                        tracing::error!("Failed to emit received message: {}", e);
                    }
                }
                Err(e) => tracing::error!("🔒 Invalid UTF-8 after decrypt: {}", e),
            }
        }
    })
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
        version: PROTOCOL_VERSION,
        message_type: MessageType::RoomJoin,
        username: username.clone(),
        user_id,
        message: format!("📍 {} moved from {} to {}", username, old_room, new_room),
        room: new_room.clone(),
        room_id: new_room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };

    {
        //update room tracking manually since server is special case
        let mut room_clients = state.room_clients.lock().await;

        //Remove from old room
        if let Some(users) = room_clients.get_mut(&old_room) {
            users.retain(|&id| id != user_id);
            tracing::info!("🔄 Removed server from room '{}'", old_room);
        }

        // Add to new room
        room_clients
            .entry(new_room.clone())
            .or_insert_with(Vec::new)
            .push(user_id);
        tracing::info!("🔄 Added server to room '{}'", new_room);

        tracing::info!("🔍 Room tracking after switch: {:?}", *room_clients);
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
            msg_clone.message_id,
        )
        .await
        {
            tracing::error!("Failed to save room join: {}", e);
        }
    });

    // Distribute room join message
    distribute_message_to_all(&app, state.inner(), &new_room, &room_join_msg, None).await;
    broadcast_user_list(&app, state.inner(), &new_room).await;
    if old_room != new_room {
        broadcast_user_list(&app, state.inner(), &old_room).await;
    }

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
        version: PROTOCOL_VERSION,
        message_type: MessageType::RoomJoin,
        username: username.clone(),
        user_id,
        message: format!("📍 {} moved from {} to {}", username, old_room, new_room),
        room: new_room.clone(),
        room_id: new_room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };

    // Send room join to server over the encrypted channel.
    send_secure_client(state.inner(), &room_join_msg)
        .await
        .map_err(|e| format!("Failed to send room join: {}", e))?;
    /*-
        - For Connect/RoomJoin/RoomLeave
        The server broadcasts these to everyone (including the sender), and your client
        listener will receive and display them.
        Emitting locally would cause a duplicate
    */
    tracing::info!("🔄 Client room switch: {} → {}", old_room, new_room);
    Ok(())
}

// A client leaves its current room (back to the lobby): tell the server so it stops
// relaying that room and notifies the other members.
#[tauri::command]
pub async fn client_leave_room(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let leave_msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::RoomLeave,
        username: username.clone(),
        user_id,
        message: format!("👋 {} left {}", username, room),
        room: room.clone(),
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };
    send_secure_client(state.inner(), &leave_msg)
        .await
        .map_err(|e| format!("Failed to send room leave: {}", e))?;
    Ok(())
}

// The host (server participant) leaves a room: update tracking locally and broadcast.
#[tauri::command]
pub async fn server_leave_room(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    user_id: u64,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let leave_msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::RoomLeave,
        username: username.clone(),
        user_id,
        message: format!("👋 {} left {}", username, room),
        room: room.clone(),
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
        message_id: Uuid::new_v4().to_string(),
    };

    {
        let mut rooms = state.room_clients.lock().await;
        if let Some(users) = rooms.get_mut(&room) {
            users.retain(|&id| id != user_id);
        }
    }

    let pool_clone = db.inner().clone();
    let msg_clone = leave_msg.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = save_message_internal(
            &pool_clone,
            msg_clone.room_id as i64,
            msg_clone.user_id as i64,
            msg_clone.message,
            "RoomLeave".to_string(),
            false,
            msg_clone.message_id,
        )
        .await
        {
            tracing::error!("Failed to save room leave: {}", e);
        }
    });

    distribute_message_to_all(&app, state.inner(), &room, &leave_msg, Some(user_id)).await;
    broadcast_user_list(&app, state.inner(), &room).await;
    Ok(())
}

// ---- Message edit / delete ----
// Edit/Delete events carry the TARGET message id in `message_id`. Clients send the
// event to the host (which applies + broadcasts via handle_server_message); the host
// participant applies + broadcasts directly.

fn edit_event(
    username: String,
    user_id: u64,
    target_id: String,
    text: String,
    room: String,
    room_id: u64,
    kind: MessageType,
) -> Message {
    Message {
        version: PROTOCOL_VERSION,
        message_type: kind,
        username,
        user_id,
        message: text,
        message_id: target_id,
        room,
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    }
}

#[tauri::command]
pub async fn client_edit_message(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    target_id: String,
    new_text: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    if new_text.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }
    let username = state.username.read().await.clone();
    let msg = edit_event(
        username,
        user_id,
        target_id,
        new_text,
        room,
        room_id,
        MessageType::Edit,
    );
    send_secure_client(state.inner(), &msg)
        .await
        .map_err(|e| format!("Failed to edit message: {}", e))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command: params map 1:1 to JS invoke args.
pub async fn server_edit_message(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    user_id: u64,
    target_id: String,
    new_text: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    if new_text.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }
    let rows = edit_message_db(db.inner(), &target_id, &new_text, user_id as i64).await?;
    if rows == 0 {
        return Err("You can only edit your own messages".to_string());
    }
    let username = state.username.read().await.clone();
    let msg = edit_event(
        username,
        user_id,
        target_id,
        new_text,
        room.clone(),
        room_id,
        MessageType::Edit,
    );
    distribute_message_to_all(&app, state.inner(), &room, &msg, None).await;
    Ok(())
}

#[tauri::command]
pub async fn client_delete_message(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    target_id: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let msg = edit_event(
        username,
        user_id,
        target_id,
        String::new(),
        room,
        room_id,
        MessageType::Delete,
    );
    send_secure_client(state.inner(), &msg)
        .await
        .map_err(|e| format!("Failed to delete message: {}", e))
}

#[tauri::command]
pub async fn server_delete_message(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    user_id: u64,
    target_id: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let rows = delete_message_db(db.inner(), &target_id, user_id as i64).await?;
    if rows == 0 {
        return Err("You can only delete your own messages".to_string());
    }
    let username = state.username.read().await.clone();
    let msg = edit_event(
        username,
        user_id,
        target_id,
        String::new(),
        room.clone(),
        room_id,
        MessageType::Delete,
    );
    distribute_message_to_all(&app, state.inner(), &room, &msg, None).await;
    Ok(())
}

// ---- Emoji reactions ----
// A client sends the toggle to the host (which applies + broadcasts the result via
// handle_server_message); the host participant applies + broadcasts directly.

#[tauri::command]
pub async fn client_toggle_reaction(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    target_id: String,
    emoji: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let msg = edit_event(
        username,
        user_id,
        target_id,
        emoji,
        room,
        room_id,
        MessageType::Reaction,
    );
    send_secure_client(state.inner(), &msg)
        .await
        .map_err(|e| format!("Failed to react: {}", e))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command: params map 1:1 to JS invoke args.
pub async fn server_toggle_reaction(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    user_id: u64,
    target_id: String,
    emoji: String,
    room: String,
    room_id: u64,
) -> Result<(), String> {
    let added = toggle_reaction_db(db.inner(), &target_id, user_id as i64, &emoji).await?;
    let username = state.username.read().await.clone();
    let mut msg = edit_event(
        username,
        user_id,
        target_id,
        emoji,
        room.clone(),
        room_id,
        MessageType::Reaction,
    );
    msg.is_emoji = added;
    distribute_message_to_all(&app, state.inner(), &room, &msg, None).await;
    Ok(())
}

/// Client → host: "I'm typing (or stopped)" in `room`. Ephemeral, best-effort —
/// a failed send is ignored so a flaky link never blocks the composer.
#[tauri::command]
pub async fn client_typing(
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    room: String,
    room_id: u64,
    typing: bool,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let mut msg = edit_event(
        username,
        user_id,
        String::new(),
        String::new(),
        room,
        room_id,
        MessageType::Typing,
    );
    msg.is_emoji = typing; // start(true)/stop(false)
    let _ = send_secure_client(state.inner(), &msg).await;
    Ok(())
}

/// Client → host: ask for the page of messages older than `before_id` in a room. The host
/// replies with a HistoryPage the client listener prepends.
#[tauri::command]
pub async fn request_history(
    state: State<'_, Arc<AppState>>,
    room: String,
    room_id: u64,
    before_id: i64,
) -> Result<(), String> {
    let msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::HistoryRequest,
        username: String::new(),
        user_id: 0,
        message: before_id.to_string(),
        message_id: Uuid::new_v4().to_string(),
        room,
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    send_secure_client(state.inner(), &msg)
        .await
        .map_err(|e| format!("Failed to request history: {}", e))
}

/// Client → host: invite `target_id` to a room. The host authorizes by the connection's
/// canonical id and notifies the invited user.
#[tauri::command]
pub async fn client_add_member(
    state: State<'_, Arc<AppState>>,
    room_id: u64,
    target_id: i64,
) -> Result<(), String> {
    let msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::AddMember,
        username: String::new(),
        user_id: 0,
        message: target_id.to_string(),
        message_id: Uuid::new_v4().to_string(),
        room: String::new(),
        room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    send_secure_client(state.inner(), &msg)
        .await
        .map_err(|e| format!("Failed to add member: {}", e))
}

/// Host participant invites a user directly against its own DB, then notifies the invitee.
#[tauri::command]
pub async fn server_add_member(
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    room_id: u64,
    target_id: i64,
    actor_id: i64,
) -> Result<(), String> {
    add_room_member_internal(db.inner(), room_id as i64, target_id, actor_id).await?;
    send_rooms_changed(state.inner(), target_id as u64).await;
    Ok(())
}

/// Host participant typing: relay straight to the room's clients (and the local UI
/// ignores its own via the sender's user_id).
#[tauri::command]
pub async fn server_typing(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    user_id: u64,
    room: String,
    room_id: u64,
    typing: bool,
) -> Result<(), String> {
    let username = state.username.read().await.clone();
    let mut msg = edit_event(
        username,
        user_id,
        String::new(),
        String::new(),
        room.clone(),
        room_id,
        MessageType::Typing,
    );
    msg.is_emoji = typing;
    distribute_message_to_all(&app, state.inner(), &room, &msg, Some(user_id)).await;
    Ok(())
}

#[tauri::command]
pub async fn get_server_info(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    let addr = state.server_addr.read().await.map(|addr| addr.to_string());
    Ok(addr)
}

/// Encrypt and send a message to one peer over its Noise transport. The transport
/// lock is held across encrypt + write so Noise nonces always reach the wire in order
/// (out-of-order frames would fail to decrypt).
async fn send_secure(
    writer: &Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    transport: &Arc<tokio::sync::Mutex<TransportState>>,
    message: &Message,
) -> Result<(), String> {
    let payload = serde_json::to_string(message).map_err(|e| e.to_string())?;
    let mut ts = transport.lock().await;
    let ciphertext = secure::encrypt(&mut ts, payload.as_bytes())?;
    let mut w = writer.lock().await;
    w.write_all(&(ciphertext.len() as u32).to_be_bytes())
        .await
        .map_err(|e| e.to_string())?;
    w.write_all(&ciphertext).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Client-side equivalent: encrypt and send to the server over the single client
/// transport. Locks transport then writer (consistent order) to keep nonces ordered.
async fn send_secure_client(state: &Arc<AppState>, message: &Message) -> Result<(), String> {
    let payload = serde_json::to_string(message).map_err(|e| e.to_string())?;
    let mut ts_guard = state.client_transport.lock().await;
    let ts = ts_guard
        .as_mut()
        .ok_or_else(|| "Not connected (no secure session)".to_string())?;
    let ciphertext = secure::encrypt(ts, payload.as_bytes())?;
    let mut w_guard = state.client_stream.lock().await;
    let w = w_guard
        .as_mut()
        .ok_or_else(|| "Not connected to server".to_string())?;
    w.write_all(&(ciphertext.len() as u32).to_be_bytes())
        .await
        .map_err(|e| e.to_string())?;
    w.write_all(&ciphertext).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn client_disconnect(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    //Read current identity from state
    let user_id_opt = { *state.user_id.read().await };
    let username = { state.username.read().await.clone() };
    let room = { state.current_room.read().await.clone() };
    let room_id_opt = { *state.current_room_id.read().await };

    // Best-effort: send an (encrypted) Disconnect to the server, then drop the session.
    let disconnect_msg = Message {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Disconnect,
        username: username.clone(),
        user_id: user_id_opt.unwrap_or(0),
        message: "client disconnect".to_string(),
        message_id: Uuid::new_v4().to_string(),
        room: room.clone(),
        room_id: room_id_opt.unwrap_or(0),
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };
    let _ = send_secure_client(state.inner(), &disconnect_msg).await;
    {
        let mut guard = state.client_stream.lock().await;
        guard.take();
    }
    {
        let mut guard = state.client_transport.lock().await;
        guard.take();
    }
    // Stop the read listener + heartbeat so they don't emit connection_lost / write to a
    // closed socket as we tear down.
    {
        let mut guard = state.client_listener.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }
    {
        let mut guard = state.client_heartbeat.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
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

    // Notify the UI on a dedicated lifecycle channel (NOT "message", which carries
    // chat payloads the frontend JSON-parses).
    let _ = app.emit("disconnected", ());

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
        version: PROTOCOL_VERSION,
        message_type: MessageType::Disconnect,
        username: host_username.clone(),
        user_id: host_user_id,
        message: "server_participant_disconnect".to_string(),
        message_id: Uuid::new_v4().to_string(),
        room: host_room.clone(),
        room_id: host_room_id,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
    };

    // Best-effort: send an encrypted disconnect notice to each client, then drop them.
    // Snapshot the writers/transports under the lock, RELEASE it, then do the network
    // sends — never hold the global server_streams Mutex across .await I/O.
    let targets: Vec<ClientLink> = {
        let mut guard = state.server_streams.lock().await;
        let snapshot = guard
            .values()
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
            .collect();
        // Dropping the ClientConnections closes their write halves.
        guard.clear();
        snapshot
    };
    for (writer, transport) in &targets {
        let _ = send_secure(writer, transport, &disconnect_msg).await;
    }
    // Clear room->clients index
    {
        let mut rooms = state.room_clients.lock().await;
        rooms.clear();
    }
    // Also clear any client-mode writer/transport if present (host may have connected out).
    {
        let mut client_w = state.client_stream.lock().await;
        client_w.take();
    }
    {
        let mut client_t = state.client_transport.lock().await;
        client_t.take();
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
    // Notify the UI that server hosting stopped, on a dedicated lifecycle channel
    // (NOT "message", which carries chat payloads the frontend JSON-parses).
    let _ = app.emit("server_stopped", ());

    Ok(())
}
