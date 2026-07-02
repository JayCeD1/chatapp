//! Wire transfer engine for attachment blobs (docs/architecture/attachments.md §2).
//!
//! Blobs move over the existing framed Noise connection in application-level chunks small
//! enough for one Noise record (~64 KB true cap — see §0 of the design). Upload is two-phase:
//! the blob transfers and persists FIRST (`AttachmentUploadStart/Chunk/Done` → `UploadOk`),
//! then the ordinary Chat frame references it by content address. Download is pull-based
//! (`AttachmentFetch` → `FetchBegin`/`Chunk` stream/`FetchDone`), membership-gated on the
//! host with the same check as history. This module owns the transfer state machines and the
//! client-side blob cache; blob persistence stays behind `blob_store` (§3a) and this code
//! never interprets blob content (E2EE forward-compat).

use crate::blob_store;
use crate::db_queries::{get_attachment_fetch_info, room_join_allowed_internal};
use crate::sockets::{
    now_secs, send_secure, send_secure_client, AppState, AttachmentRef, Message, MessageType,
    PROTOCOL_VERSION,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;
use uuid::Uuid;

/// Hard cap on a single attachment (design §2). Bounds host memory (uploads buffer in RAM)
/// and DB growth; revisit with real-world usage.
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
/// Raw bytes per wire chunk: base64(45,056) ≈ 60 KB + JSON wrapper + envelope stays under
/// the 65,519-byte Noise plaintext cap with headroom. Independent of the storage row size.
pub const ATTACHMENT_CHUNK_BYTES: usize = 44 * 1024;
/// Cap refs per Chat frame (UI + frame-size sanity).
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 5;
/// One upload per connection, bounded uploads host-wide (worst-case buffering = 4 × 25 MiB).
const MAX_UPLOADS_PER_HOST: usize = 4;
/// Concurrent blob downloads per connection / host-wide (§9a trap 8: read_blob materializes
/// up to 25 MiB per stream, so the host-wide cap bounds total memory).
const MAX_DOWNLOADS_PER_CONN: usize = 2;
const MAX_DOWNLOADS_PER_HOST: usize = 8;
/// Per-connection inbound chunk budget: chunks bypass the message-count rate limiter and are
/// governed by this byte bucket instead (counted on the wire payload, base64 inclusive).
const CHUNK_RATE_BYTES_PER_SEC: f64 = 4.0 * 1024.0 * 1024.0;
const CHUNK_BURST_BYTES: f64 = 8.0 * 1024.0 * 1024.0;
/// Upper bound on one chunk's base64 payload, enforced before decoding (allocation guard).
const MAX_CHUNK_B64_LEN: usize = 61_440;
/// How long the client upload driver waits for each host control reply.
const CONTROL_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Client-side in-memory blob cache budget (session-scoped LRU; re-fetching on LAN is cheap).
const CLIENT_CACHE_MAX_BYTES: usize = 256 * 1024 * 1024;

// ---- Wire payloads (JSON carried in the envelope's `message` field, like HistoryRequest) ----

#[derive(Serialize, Deserialize)]
struct UploadStartPayload {
    sha256: String,
    size: usize,
}

#[derive(Serialize, Deserialize)]
struct ShaPayload {
    sha256: String,
}

#[derive(Serialize, Deserialize)]
struct ChunkPayload {
    sha256: String,
    seq: u64,
    data: String, // base64
}

#[derive(Serialize, Deserialize)]
struct FetchPayload {
    attachment_id: String,
}

#[derive(Serialize, Deserialize)]
struct FetchBeginPayload {
    attachment_id: String,
    sha256: String,
    size: usize,
}

#[derive(Serialize, Deserialize)]
struct FetchDonePayload {
    attachment_id: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
struct ErrorPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attachment_id: Option<String>,
    reason: String,
}

/// Progress event emitted to the local UI while bytes move.
#[derive(Serialize, Clone)]
struct ProgressEvent<'a> {
    sha256: &'a str,
    direction: &'a str, // "upload" | "download"
    transferred: usize,
    total: usize,
}

// ---- Byte-bucket rate limiter (chunks only; see the read loop in sockets.rs) ----

/// Token bucket denominated in bytes. Same refill shape as the message-count RateLimiter,
/// but consuming the frame's payload size so a 25 MiB upload can't starve chat frames of
/// their budget nor flood the host.
pub struct ByteRateLimiter {
    tokens: f64,
    last: tokio::time::Instant,
}

impl ByteRateLimiter {
    pub fn new(now: tokio::time::Instant) -> Self {
        Self {
            tokens: CHUNK_BURST_BYTES,
            last: now,
        }
    }

    pub fn allow(&mut self, now: tokio::time::Instant, bytes: usize) -> bool {
        self.tokens = (self.tokens
            + now.duration_since(self.last).as_secs_f64() * CHUNK_RATE_BYTES_PER_SEC)
            .min(CHUNK_BURST_BYTES);
        self.last = now;
        let cost = bytes as f64;
        if self.tokens < cost {
            return false;
        }
        self.tokens -= cost;
        true
    }
}

// ---- Host-side state ----

struct UploadSession {
    sha256: String,
    declared: usize,
    buf: Vec<u8>,
    next_seq: u64,
}

#[derive(Default)]
struct DownloadSlots {
    per_conn: HashMap<u64, usize>,
    total: usize,
}

/// Host-side transfer state, keyed by the connection's canonical user id. Upload sessions
/// are dropped with the connection (clean_client → `drop_upload_session`); download slots
/// release when their streaming task ends (a dead socket fails the next send immediately).
#[derive(Default)]
pub struct HostTransfers {
    uploads: tokio::sync::Mutex<HashMap<u64, UploadSession>>,
    downloads: tokio::sync::Mutex<DownloadSlots>,
}

// ---- Client-side state ----

enum UploadSignal {
    Ready,
    Complete,
    Failed(String),
}

struct ActiveUpload {
    sha256: String,
    notify: tokio::sync::mpsc::Sender<UploadSignal>,
}

struct DownloadAssembly {
    attachment_id: String,
    expected: usize,
    buf: Vec<u8>,
    next_seq: u64,
}

/// Session-scoped LRU blob cache (design §3): fetched blobs live in memory only; no client
/// DB rows, no disk. Values are Arc'd so handing bytes to the IPC layer never copies.
#[derive(Default)]
struct BlobCache {
    map: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    bytes: usize,
}

impl BlobCache {
    fn insert(&mut self, sha256: String, bytes: Vec<u8>) {
        if self.map.contains_key(&sha256) {
            return;
        }
        self.bytes += bytes.len();
        self.map.insert(sha256.clone(), Arc::new(bytes));
        self.order.push_back(sha256);
        while self.bytes > CLIENT_CACHE_MAX_BYTES && self.order.len() > 1 {
            if let Some(oldest) = self.order.pop_front() {
                if let Some(evicted) = self.map.remove(&oldest) {
                    self.bytes -= evicted.len();
                }
            }
        }
    }

    fn get(&mut self, sha256: &str) -> Option<Arc<Vec<u8>>> {
        let hit = self.map.get(sha256).cloned();
        if hit.is_some() {
            // Refresh recency so hot blobs survive eviction.
            self.order.retain(|s| s != sha256);
            self.order.push_back(sha256.to_string());
        }
        hit
    }
}

/// Client-side transfer state: one upload driver at a time, in-flight download assemblies
/// keyed by sha256, and the LRU byte cache.
#[derive(Default)]
pub struct ClientTransfers {
    upload: tokio::sync::Mutex<Option<ActiveUpload>>,
    downloads: tokio::sync::Mutex<HashMap<String, DownloadAssembly>>,
    cache: tokio::sync::Mutex<BlobCache>,
}

// ---- Shared helpers ----

fn control_frame(msg_type: MessageType, payload: String) -> Message {
    Message {
        version: PROTOCOL_VERSION,
        message_type: msg_type,
        username: String::new(),
        user_id: 0,
        message: payload,
        message_id: Uuid::new_v4().to_string(),
        room: String::new(),
        room_id: 0,
        created_at: now_secs(),
        is_emoji: false,
        email: None,
        attachments: None,
        features: None,
    }
}

fn is_hex_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn emit_progress(app: &tauri::AppHandle, sha256: &str, direction: &str, done: usize, total: usize) {
    let _ = app.emit(
        "attachment_progress",
        ProgressEvent {
            sha256,
            direction,
            transferred: done,
            total,
        },
    );
}

/// Send one control frame to a connected client (snapshot the link, then send lock-free).
async fn send_control(state: &Arc<AppState>, user_id: u64, msg: &Message) {
    let conn = {
        let streams = state.server_streams.lock().await;
        streams
            .get(&user_id)
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
    };
    if let Some((writer, transport)) = conn {
        let _ = send_secure(&writer, &transport, msg).await;
    }
}

async fn send_attachment_error(
    state: &Arc<AppState>,
    user_id: u64,
    sha256: Option<&str>,
    attachment_id: Option<&str>,
    reason: &str,
) {
    let payload = serde_json::to_string(&ErrorPayload {
        sha256: sha256.map(str::to_string),
        attachment_id: attachment_id.map(str::to_string),
        reason: reason.to_string(),
    })
    .unwrap_or_default();
    send_control(
        state,
        user_id,
        &control_frame(MessageType::AttachmentError, payload),
    )
    .await;
}

// ---- Host-side handlers (called from handle_server_message; user_id is connection-bound) ----

/// `AttachmentUploadStart {sha256, size}` — validate, dedup fast-path, reserve the session.
pub async fn handle_upload_start(
    state: &Arc<AppState>,
    pool: &SqlitePool,
    user_id: u64,
    payload: &str,
) {
    let Ok(req) = serde_json::from_str::<UploadStartPayload>(payload) else {
        send_attachment_error(state, user_id, None, None, "Malformed upload request").await;
        return;
    };
    if !is_hex_sha256(&req.sha256) {
        send_attachment_error(state, user_id, None, None, "Invalid content hash").await;
        return;
    }
    if req.size == 0 || req.size > MAX_ATTACHMENT_BYTES {
        send_attachment_error(
            state,
            user_id,
            Some(&req.sha256),
            None,
            &format!(
                "Attachment exceeds {} MB limit",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ),
        )
        .await;
        return;
    }

    // Dedup fast-path: identical bytes already stored and complete → no transfer needed.
    match blob_store::complete_blob_size(pool, &req.sha256).await {
        Ok(Some(_)) => {
            let payload =
                serde_json::to_string(&ShaPayload { sha256: req.sha256 }).unwrap_or_default();
            send_control(
                state,
                user_id,
                &control_frame(MessageType::AttachmentUploadOk, payload),
            )
            .await;
            return;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("Upload dedup check failed: {}", e);
            send_attachment_error(state, user_id, Some(&req.sha256), None, "Storage error").await;
            return;
        }
    }

    {
        let mut uploads = state.attachments_host.uploads.lock().await;
        // A new Start from the same connection supersedes its previous (dead) attempt, so a
        // crashed driver can always retry; only OTHER connections count toward the cap.
        let others = uploads.keys().filter(|&&uid| uid != user_id).count();
        if others >= MAX_UPLOADS_PER_HOST {
            drop(uploads);
            send_attachment_error(
                state,
                user_id,
                Some(&req.sha256),
                None,
                "Host is busy with other uploads — try again shortly",
            )
            .await;
            return;
        }
        uploads.insert(
            user_id,
            UploadSession {
                sha256: req.sha256.clone(),
                declared: req.size,
                buf: Vec::with_capacity(req.size),
                next_seq: 0,
            },
        );
    }
    let payload = serde_json::to_string(&ShaPayload { sha256: req.sha256 }).unwrap_or_default();
    send_control(
        state,
        user_id,
        &control_frame(MessageType::AttachmentUploadReady, payload),
    )
    .await;
}

/// `AttachmentChunk {sha256, seq, data}` (upload direction) — sequential append with caps.
/// Chunks with no accepted session are dropped silently (flood-safe: they already paid the
/// byte bucket in the read loop).
pub async fn handle_upload_chunk(state: &Arc<AppState>, user_id: u64, payload: &str) {
    let Ok(chunk) = serde_json::from_str::<ChunkPayload>(payload) else {
        return;
    };
    if chunk.data.len() > MAX_CHUNK_B64_LEN {
        abort_upload(state, user_id, "Oversized chunk").await;
        return;
    }
    let Ok(bytes) = BASE64.decode(&chunk.data) else {
        abort_upload(state, user_id, "Undecodable chunk").await;
        return;
    };

    let failure = {
        let mut uploads = state.attachments_host.uploads.lock().await;
        let Some(session) = uploads.get_mut(&user_id) else {
            return; // no accepted upload in progress — drop
        };
        if chunk.sha256 != session.sha256 {
            None::<&str> // chunk for a stale attempt — drop, don't kill the live session
        } else if chunk.seq != session.next_seq {
            Some("Out-of-order chunk")
        } else if bytes.is_empty() || bytes.len() > ATTACHMENT_CHUNK_BYTES {
            Some("Bad chunk size")
        } else if session.buf.len() + bytes.len() > session.declared {
            Some("Upload exceeds its declared size")
        } else {
            session.buf.extend_from_slice(&bytes);
            session.next_seq += 1;
            return;
        }
    };
    if let Some(reason) = failure {
        abort_upload(state, user_id, reason).await;
    }
}

/// `AttachmentUploadDone {sha256}` — verify the declared hash BEFORE persisting (§9a trap 7:
/// persist-then-check would store mismatched garbage under its true hash — a disk-filler),
/// then hand the bytes to the blob store and confirm.
pub async fn handle_upload_done(
    state: &Arc<AppState>,
    pool: &SqlitePool,
    user_id: u64,
    payload: &str,
) {
    let Ok(done) = serde_json::from_str::<ShaPayload>(payload) else {
        return;
    };
    let session = {
        let mut uploads = state.attachments_host.uploads.lock().await;
        match uploads.get(&user_id) {
            Some(s) if s.sha256 == done.sha256 => uploads.remove(&user_id),
            _ => None,
        }
    };
    let Some(session) = session else {
        send_attachment_error(
            state,
            user_id,
            Some(&done.sha256),
            None,
            "No upload in progress",
        )
        .await;
        return;
    };

    if session.buf.len() != session.declared
        || blob_store::hex_sha256(&session.buf) != session.sha256
    {
        send_attachment_error(
            state,
            user_id,
            Some(&session.sha256),
            None,
            "Upload did not match its declared hash — retry",
        )
        .await;
        return;
    }

    match blob_store::store_blob(pool, &session.buf).await {
        Ok(stored_sha) if stored_sha == session.sha256 => {
            let payload = serde_json::to_string(&ShaPayload {
                sha256: session.sha256,
            })
            .unwrap_or_default();
            send_control(
                state,
                user_id,
                &control_frame(MessageType::AttachmentUploadOk, payload),
            )
            .await;
        }
        Ok(_) | Err(_) => {
            send_attachment_error(state, user_id, Some(&session.sha256), None, "Storage error")
                .await;
        }
    }
}

async fn abort_upload(state: &Arc<AppState>, user_id: u64, reason: &str) {
    let sha = {
        let mut uploads = state.attachments_host.uploads.lock().await;
        uploads.remove(&user_id).map(|s| s.sha256)
    };
    tracing::warn!("Aborted upload from {}: {}", user_id, reason);
    send_attachment_error(state, user_id, sha.as_deref(), None, reason).await;
}

/// Clear a disconnecting connection's upload session (clean_client hook). Download slots
/// are NOT force-released here: their streaming tasks fail the next send on the dead socket
/// and release themselves, so a forced release would double-decrement.
pub async fn drop_upload_session(state: &Arc<AppState>, user_id: u64) {
    let mut uploads = state.attachments_host.uploads.lock().await;
    uploads.remove(&user_id);
}

/// `AttachmentFetch {attachment_id}` — gate by room membership (same check as history), then
/// stream the blob back as a spawned, sequentially-paced chunk task.
pub async fn handle_fetch(state: &Arc<AppState>, pool: &SqlitePool, user_id: u64, payload: &str) {
    let Ok(req) = serde_json::from_str::<FetchPayload>(payload) else {
        return;
    };
    if req.attachment_id.is_empty() || req.attachment_id.len() > 64 {
        return;
    }

    // Resolve id → (sha, size, room); a deleted message's attachments resolve to None.
    let info = match get_attachment_fetch_info(pool, &req.attachment_id).await {
        Ok(Some(info)) => info,
        Ok(None) => {
            send_attachment_error(
                state,
                user_id,
                None,
                Some(&req.attachment_id),
                "Attachment is no longer available",
            )
            .await;
            return;
        }
        Err(e) => {
            tracing::error!("Fetch lookup failed: {}", e);
            send_attachment_error(
                state,
                user_id,
                None,
                Some(&req.attachment_id),
                "Storage error",
            )
            .await;
            return;
        }
    };

    // Membership gate on the connection-bound id — fetch is keyed by attachment id, never by
    // sha, so knowing a content hash grants nothing (design §6).
    if !room_join_allowed_internal(pool, user_id as i64, info.room_id)
        .await
        .unwrap_or(false)
    {
        tracing::warn!(
            "Denied attachment {} to non-member {}",
            req.attachment_id,
            user_id
        );
        send_attachment_error(
            state,
            user_id,
            None,
            Some(&req.attachment_id),
            "Not authorized",
        )
        .await;
        return;
    }

    // Concurrency slots: per-connection and host-wide (bounds read_blob's 25 MiB buffers).
    {
        let mut slots = state.attachments_host.downloads.lock().await;
        let mine = slots.per_conn.get(&user_id).copied().unwrap_or(0);
        if mine >= MAX_DOWNLOADS_PER_CONN || slots.total >= MAX_DOWNLOADS_PER_HOST {
            drop(slots);
            send_attachment_error(
                state,
                user_id,
                None,
                Some(&req.attachment_id),
                "Too many concurrent downloads — try again shortly",
            )
            .await;
            return;
        }
        *slots.per_conn.entry(user_id).or_insert(0) += 1;
        slots.total += 1;
    }

    let state = Arc::clone(state);
    let pool = pool.clone();
    tauri::async_runtime::spawn(async move {
        stream_blob_to_client(&state, &pool, user_id, &req.attachment_id, &info.sha256).await;
        let mut slots = state.attachments_host.downloads.lock().await;
        if let Some(c) = slots.per_conn.get_mut(&user_id) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                slots.per_conn.remove(&user_id);
            }
        }
        slots.total = slots.total.saturating_sub(1);
    });
}

/// The streaming half of a fetch: read the blob (single snapshot), then FetchBegin → chunk
/// loop → FetchDone over the requester's connection. Sequential awaited writes give natural
/// TCP backpressure; the writer mutex is released between chunks so chat frames interleave.
async fn stream_blob_to_client(
    state: &Arc<AppState>,
    pool: &SqlitePool,
    user_id: u64,
    attachment_id: &str,
    sha256: &str,
) {
    let bytes = match blob_store::read_blob(pool, sha256).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            // Raced a GC — the blob is cleanly gone, not corrupt (§9a trap 8).
            send_attachment_error(
                state,
                user_id,
                Some(sha256),
                Some(attachment_id),
                "Attachment is no longer available",
            )
            .await;
            return;
        }
        Err(e) => {
            tracing::error!("read_blob({}) failed: {}", sha256, e);
            send_attachment_error(
                state,
                user_id,
                Some(sha256),
                Some(attachment_id),
                "Storage error",
            )
            .await;
            return;
        }
    };

    // Snapshot the link once; if the connection is replaced mid-stream the sends fail and
    // the transfer aborts (the client's assembly times out with its connection).
    let conn = {
        let streams = state.server_streams.lock().await;
        streams
            .get(&user_id)
            .map(|c| (Arc::clone(&c.writer), Arc::clone(&c.transport)))
    };
    let Some((writer, transport)) = conn else {
        return;
    };

    let begin = serde_json::to_string(&FetchBeginPayload {
        attachment_id: attachment_id.to_string(),
        sha256: sha256.to_string(),
        size: bytes.len(),
    })
    .unwrap_or_default();
    if send_secure(
        &writer,
        &transport,
        &control_frame(MessageType::AttachmentFetchBegin, begin),
    )
    .await
    .is_err()
    {
        return;
    }

    for (seq, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
        let payload = serde_json::to_string(&ChunkPayload {
            sha256: sha256.to_string(),
            seq: seq as u64,
            data: BASE64.encode(chunk),
        })
        .unwrap_or_default();
        if send_secure(
            &writer,
            &transport,
            &control_frame(MessageType::AttachmentChunk, payload),
        )
        .await
        .is_err()
        {
            return;
        }
    }

    let done = serde_json::to_string(&FetchDonePayload {
        attachment_id: attachment_id.to_string(),
        sha256: sha256.to_string(),
    })
    .unwrap_or_default();
    let _ = send_secure(
        &writer,
        &transport,
        &control_frame(MessageType::AttachmentFetchDone, done),
    )
    .await;
}

/// Validate the attachment refs on an inbound Chat frame BEFORE persisting or relaying
/// (§9a trap 1: the relay clones the whole message, so unvalidated refs would propagate).
/// Every ref must point at a complete stored blob whose size matches the claim.
pub async fn validate_chat_attachments(
    pool: &SqlitePool,
    refs: &[AttachmentRef],
) -> Result<(), String> {
    if refs.is_empty() || refs.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(format!(
            "A message can carry 1–{} attachments",
            MAX_ATTACHMENTS_PER_MESSAGE
        ));
    }
    let mut seen_ids = std::collections::HashSet::new();
    for r in refs {
        if r.id.is_empty() || r.id.len() > 64 || !seen_ids.insert(r.id.as_str()) {
            return Err("Invalid attachment id".to_string());
        }
        if !is_hex_sha256(&r.sha256) {
            return Err("Invalid attachment hash".to_string());
        }
        if r.name.is_empty() || r.name.chars().count() > 255 || r.mime.len() > 128 {
            return Err("Invalid attachment metadata".to_string());
        }
        if r.size == 0 || r.size as usize > MAX_ATTACHMENT_BYTES {
            return Err("Invalid attachment size".to_string());
        }
        match blob_store::complete_blob_size(pool, &r.sha256).await {
            Ok(Some(stored)) if stored == r.size as i64 => {}
            Ok(_) => return Err("Attachment was not uploaded".to_string()),
            Err(e) => {
                tracing::error!("Attachment validation failed: {}", e);
                return Err("Storage error".to_string());
            }
        }
    }
    Ok(())
}

// ---- Client-side driver + frame interceptor ----
// The upload/fetch drivers become Tauri commands in Phase 4; until then they're only
// exercised by the loopback integration tests.

/// Upload `bytes` to the host over the live client connection and return the content sha.
/// One upload at a time; progress is emitted as `attachment_progress` events.
#[allow(dead_code)] // TODO(attachments Phase 4): wired by the upload command.
pub async fn client_upload_attachment(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    bytes: Vec<u8>,
) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("Cannot attach an empty file".to_string());
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "Files up to {} MB are supported",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        ));
    }
    let sha256 = blob_store::hex_sha256(&bytes);

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    {
        let mut slot = state.attachments_client.upload.lock().await;
        if slot.is_some() {
            return Err("Another upload is already in progress".to_string());
        }
        *slot = Some(ActiveUpload {
            sha256: sha256.clone(),
            notify: tx,
        });
    }

    let result = drive_upload(app, state, &sha256, &bytes, &mut rx).await;

    // Always release the driver slot, success or failure.
    {
        let mut slot = state.attachments_client.upload.lock().await;
        *slot = None;
    }

    if result.is_ok() {
        // Seed the local cache so the sender's own message previews instantly.
        let mut cache = state.attachments_client.cache.lock().await;
        cache.insert(sha256.clone(), bytes);
    }
    result.map(|_| sha256)
}

#[allow(dead_code)]
async fn drive_upload(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    sha256: &str,
    bytes: &[u8],
    rx: &mut tokio::sync::mpsc::Receiver<UploadSignal>,
) -> Result<(), String> {
    let start = serde_json::to_string(&UploadStartPayload {
        sha256: sha256.to_string(),
        size: bytes.len(),
    })
    .map_err(|e| e.to_string())?;
    send_secure_client(
        state,
        &control_frame(MessageType::AttachmentUploadStart, start),
    )
    .await?;

    match await_signal(rx).await? {
        UploadSignal::Complete => return Ok(()), // dedup fast-path: host already has it
        UploadSignal::Ready => {}
        UploadSignal::Failed(reason) => return Err(reason),
    }

    let total = bytes.len();
    let mut sent = 0usize;
    for (seq, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
        let payload = serde_json::to_string(&ChunkPayload {
            sha256: sha256.to_string(),
            seq: seq as u64,
            data: BASE64.encode(chunk),
        })
        .map_err(|e| e.to_string())?;
        send_secure_client(state, &control_frame(MessageType::AttachmentChunk, payload)).await?;
        sent += chunk.len();
        emit_progress(app, sha256, "upload", sent, total);
    }

    let done = serde_json::to_string(&ShaPayload {
        sha256: sha256.to_string(),
    })
    .map_err(|e| e.to_string())?;
    send_secure_client(
        state,
        &control_frame(MessageType::AttachmentUploadDone, done),
    )
    .await?;

    match await_signal(rx).await? {
        UploadSignal::Complete => Ok(()),
        UploadSignal::Ready => Err("Unexpected host reply".to_string()),
        UploadSignal::Failed(reason) => Err(reason),
    }
}

#[allow(dead_code)]
async fn await_signal(
    rx: &mut tokio::sync::mpsc::Receiver<UploadSignal>,
) -> Result<UploadSignal, String> {
    match tokio::time::timeout(CONTROL_REPLY_TIMEOUT, rx.recv()).await {
        Ok(Some(sig)) => Ok(sig),
        Ok(None) => Err("Connection lost during upload".to_string()),
        Err(_) => Err("Host did not respond — try again".to_string()),
    }
}

/// Request an attachment's bytes from the host (no-op if already cached — the `attachment_ready`
/// event fires either way). The bytes land in the client cache; Phase 4's command reads them out.
#[allow(dead_code)] // TODO(attachments Phase 4): wired by the fetch command.
pub async fn client_fetch_attachment(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    attachment_id: String,
    sha256: String,
) -> Result<(), String> {
    if !is_hex_sha256(&sha256) {
        return Err("Invalid attachment hash".to_string());
    }
    {
        let mut cache = state.attachments_client.cache.lock().await;
        if cache.get(&sha256).is_some() {
            let _ = app.emit(
                "attachment_ready",
                serde_json::json!({ "attachment_id": attachment_id, "sha256": sha256 }),
            );
            return Ok(());
        }
    }
    {
        let downloads = state.attachments_client.downloads.lock().await;
        if downloads.contains_key(&sha256) {
            return Ok(()); // already fetching this content; the ready event will cover it
        }
    }
    let payload =
        serde_json::to_string(&FetchPayload { attachment_id }).map_err(|e| e.to_string())?;
    send_secure_client(state, &control_frame(MessageType::AttachmentFetch, payload)).await
}

/// Read a fetched/uploaded blob out of the client cache (Phase 4's `get_attachment_bytes`).
#[allow(dead_code)] // TODO(attachments Phase 4): wired by the bytes command.
pub async fn cached_blob(state: &Arc<AppState>, sha256: &str) -> Option<Arc<Vec<u8>>> {
    let mut cache = state.attachments_client.cache.lock().await;
    cache.get(sha256)
}

/// Reset all client-side transfer state (disconnect/reconnect): fail the active upload,
/// drop in-flight assemblies. The cache survives — content-addressed bytes stay valid.
pub async fn reset_client(state: &Arc<AppState>) {
    let upload = {
        let mut slot = state.attachments_client.upload.lock().await;
        slot.take()
    };
    if let Some(active) = upload {
        let _ = active
            .notify
            .send(UploadSignal::Failed("Connection lost".to_string()))
            .await;
    }
    let mut downloads = state.attachments_client.downloads.lock().await;
    downloads.clear();
}

/// Intercept attachment protocol frames on the client read path. Returns true when the frame
/// was consumed (protocol-internal — never emitted to the UI; chunk frames especially would
/// flood the webview).
pub async fn intercept_client_frame(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    message: &Message,
) -> bool {
    match message.message_type {
        MessageType::AttachmentUploadReady => {
            signal_upload(state, &message.message, UploadSignal::Ready).await;
            true
        }
        MessageType::AttachmentUploadOk => {
            signal_upload(state, &message.message, UploadSignal::Complete).await;
            true
        }
        MessageType::AttachmentFetchBegin => {
            if let Ok(begin) = serde_json::from_str::<FetchBeginPayload>(&message.message) {
                if begin.size > 0 && begin.size <= MAX_ATTACHMENT_BYTES {
                    let mut downloads = state.attachments_client.downloads.lock().await;
                    downloads.insert(
                        begin.sha256.clone(),
                        DownloadAssembly {
                            attachment_id: begin.attachment_id,
                            expected: begin.size,
                            buf: Vec::with_capacity(begin.size),
                            next_seq: 0,
                        },
                    );
                }
            }
            true
        }
        MessageType::AttachmentChunk => {
            handle_download_chunk(app, state, &message.message).await;
            true
        }
        MessageType::AttachmentFetchDone => {
            finish_download(app, state, &message.message).await;
            true
        }
        MessageType::AttachmentError => {
            if let Ok(err) = serde_json::from_str::<ErrorPayload>(&message.message) {
                // Route to the active upload if it matches; otherwise it's a download failure.
                if let Some(sha) = &err.sha256 {
                    let matches_upload = {
                        let slot = state.attachments_client.upload.lock().await;
                        slot.as_ref().is_some_and(|u| &u.sha256 == sha)
                    };
                    if matches_upload {
                        signal_upload_by_sha(state, sha, UploadSignal::Failed(err.reason.clone()))
                            .await;
                        return true;
                    }
                    let mut downloads = state.attachments_client.downloads.lock().await;
                    downloads.remove(sha);
                }
                let _ = app.emit(
                    "attachment_failed",
                    serde_json::json!({
                        "sha256": err.sha256,
                        "attachment_id": err.attachment_id,
                        "reason": err.reason,
                    }),
                );
            }
            true
        }
        _ => false,
    }
}

async fn signal_upload(state: &Arc<AppState>, payload: &str, signal: UploadSignal) {
    if let Ok(p) = serde_json::from_str::<ShaPayload>(payload) {
        signal_upload_by_sha(state, &p.sha256, signal).await;
    }
}

async fn signal_upload_by_sha(state: &Arc<AppState>, sha256: &str, signal: UploadSignal) {
    let notify = {
        let slot = state.attachments_client.upload.lock().await;
        slot.as_ref()
            .filter(|u| u.sha256 == sha256)
            .map(|u| u.notify.clone())
    };
    if let Some(tx) = notify {
        let _ = tx.send(signal).await;
    }
}

async fn handle_download_chunk(app: &tauri::AppHandle, state: &Arc<AppState>, payload: &str) {
    let Ok(chunk) = serde_json::from_str::<ChunkPayload>(payload) else {
        return;
    };
    if chunk.data.len() > MAX_CHUNK_B64_LEN {
        let mut downloads = state.attachments_client.downloads.lock().await;
        downloads.remove(&chunk.sha256);
        return;
    }
    let Ok(bytes) = BASE64.decode(&chunk.data) else {
        let mut downloads = state.attachments_client.downloads.lock().await;
        downloads.remove(&chunk.sha256);
        return;
    };

    let progress = {
        let mut downloads = state.attachments_client.downloads.lock().await;
        let Some(asm) = downloads.get_mut(&chunk.sha256) else {
            return; // unsolicited chunk — drop
        };
        if chunk.seq != asm.next_seq || asm.buf.len() + bytes.len() > asm.expected {
            downloads.remove(&chunk.sha256);
            return;
        }
        asm.buf.extend_from_slice(&bytes);
        asm.next_seq += 1;
        (asm.buf.len(), asm.expected)
    };
    emit_progress(app, &chunk.sha256, "download", progress.0, progress.1);
}

async fn finish_download(app: &tauri::AppHandle, state: &Arc<AppState>, payload: &str) {
    let Ok(done) = serde_json::from_str::<FetchDonePayload>(payload) else {
        return;
    };
    let asm = {
        let mut downloads = state.attachments_client.downloads.lock().await;
        downloads.remove(&done.sha256)
    };
    let Some(asm) = asm else {
        return;
    };

    // Integrity: the assembled bytes must hash to the advertised content address.
    if asm.buf.len() != asm.expected || blob_store::hex_sha256(&asm.buf) != done.sha256 {
        let _ = app.emit(
            "attachment_failed",
            serde_json::json!({
                "sha256": done.sha256,
                "attachment_id": asm.attachment_id,
                "reason": "File was corrupted in transfer — retry",
            }),
        );
        return;
    }

    {
        let mut cache = state.attachments_client.cache.lock().await;
        cache.insert(done.sha256.clone(), asm.buf);
    }
    let _ = app.emit(
        "attachment_ready",
        serde_json::json!({
            "attachment_id": asm.attachment_id,
            "sha256": done.sha256,
        }),
    );
}
