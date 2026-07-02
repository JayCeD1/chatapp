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
use tauri::{Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tokio::io::AsyncWriteExt;
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
/// Per-frame send timeout while streaming a download, so a black-holed TCP write (a peer that
/// vanished without a FIN) can't pin a download slot + its 25 MiB buffer for minutes until
/// TCP retransmission gives up (§9a trap 13).
const STREAM_SEND_TIMEOUT: Duration = Duration::from_secs(30);
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

/// Client-side transfer state: one upload driver at a time, fetches we've actually asked
/// for (`pending`), in-flight download assemblies keyed by sha256, and the LRU byte cache.
#[derive(Default)]
pub struct ClientTransfers {
    upload: tokio::sync::Mutex<Option<ActiveUpload>>,
    // attachment_id → expected sha256 of fetches this client sent. A FetchBegin is honored
    // ONLY for an entry here (§9a trap 10: never trust an unsolicited FetchBegin), and
    // pending + downloads together are capped so a broken host can't run the client's
    // memory up N × 25 MiB.
    pending: tokio::sync::Mutex<HashMap<String, String>>,
    downloads: tokio::sync::Mutex<HashMap<String, DownloadAssembly>>,
    cache: tokio::sync::Mutex<BlobCache>,
}

/// Cap on pending + in-flight download assemblies client-side (§9a trap 10).
const MAX_CLIENT_ASSEMBLIES: usize = 4;

/// Register a fetch the client is about to send. `Ok(true)` = send the Fetch frame;
/// `Ok(false)` = the same content is already pending/in-flight (the eventual
/// `attachment_ready`/`attachment_failed` event carries the sha, so the UI can key on it);
/// `Err` = the client is at its concurrent-download cap.
async fn register_pending_fetch(
    state: &Arc<AppState>,
    attachment_id: &str,
    sha256: &str,
) -> Result<bool, String> {
    let mut pending = state.attachments_client.pending.lock().await;
    let downloads = state.attachments_client.downloads.lock().await;
    if pending.contains_key(attachment_id)
        || pending.values().any(|s| s == sha256)
        || downloads.contains_key(sha256)
    {
        return Ok(false);
    }
    if pending.len() + downloads.len() >= MAX_CLIENT_ASSEMBLIES {
        return Err("Too many downloads in progress — try again shortly".to_string());
    }
    pending.insert(attachment_id.to_string(), sha256.to_string());
    Ok(true)
}

/// Honor a FetchBegin only if it answers a fetch we sent (matching attachment id AND sha).
/// Returns true when an assembly was created. The pending slot is consumed either way once
/// the id matches, so a host replying with a bogus size can't pin a cap slot forever.
async fn accept_fetch_begin(state: &Arc<AppState>, begin: &FetchBeginPayload) -> bool {
    {
        let mut pending = state.attachments_client.pending.lock().await;
        match pending.get(&begin.attachment_id) {
            // Requested this id AND the sha matches — consume the slot and proceed.
            Some(expected) if *expected == begin.sha256 => {
                pending.remove(&begin.attachment_id);
            }
            // Requested this id but the host answered with a DIFFERENT sha (content
            // substitution attempt): consume the slot so a buggy/hostile host can't pin
            // it, and drop the Begin.
            Some(_) => {
                pending.remove(&begin.attachment_id);
                return false;
            }
            // Never requested this id — unsolicited; drop without touching the registry.
            None => return false,
        }
    }
    if begin.size == 0 || begin.size > MAX_ATTACHMENT_BYTES {
        return false;
    }
    let mut downloads = state.attachments_client.downloads.lock().await;
    if downloads.contains_key(&begin.sha256) {
        return false; // an assembly for this content is already in flight — don't clobber it
    }
    downloads.insert(
        begin.sha256.clone(),
        DownloadAssembly {
            attachment_id: begin.attachment_id.clone(),
            expected: begin.size,
            buf: Vec::with_capacity(begin.size),
            next_seq: 0,
        },
    );
    true
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

    // Dedup fast-path: identical bytes already stored → reply UploadOk (no transfer) — but
    // ONLY if the sender may reference this content (§9a item 9). Otherwise fall through to a
    // normal upload, so "the blob exists but isn't yours" is indistinguishable from "the blob
    // doesn't exist": both take the Ok(None)/unauthorized branch and reply UploadReady after
    // the SAME work. `may_reference_sha` is evaluated UNCONDITIONALLY first so the two negative
    // cases cost the same (no timing oracle). An attacker who only knows a hash thus learns
    // nothing and can complete an upload only by actually possessing the bytes.
    let may_ref = may_reference_sha(pool, user_id as i64, &req.sha256)
        .await
        .unwrap_or(false);
    match blob_store::complete_blob_size(pool, &req.sha256).await {
        Ok(Some(_)) if may_ref => {
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
        Ok(_) => {}
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
            // Record proof-of-possession so this user may reference the content (§9a item 9).
            // Done on EVERY successful completion — including when store_blob deduped an
            // existing blob — so a second uploader of the same bytes still gets their own row.
            // Non-fatal on error: worst case the user re-uploads to reference it.
            if let Err(e) = blob_store::record_uploader(pool, &session.sha256, user_id as i64).await
            {
                tracing::error!("Failed to record uploader possession: {}", e);
            }
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

/// Send one frame with a timeout (§9a trap 13). Returns false on error OR timeout, so the
/// streaming caller aborts and its download slot is released instead of pinned on a dead
/// socket.
///
/// A timeout needs care: `tokio::time::timeout` cancels by DROPPING the `send_secure` future,
/// which is not cancellation-safe — `secure::encrypt` has already advanced the Noise send
/// nonce (and a partial frame may be on the wire), so the connection is now desynced and
/// every later frame to this peer would fail the AEAD tag. Rather than leave it live-but-
/// corrupted in `server_streams`, shut the write half down so it's a clean disconnect: after
/// a 30s stall on a single ~60 KB frame the peer is almost certainly gone/asleep, and the
/// client reconnects. (A plain send error means the socket is already broken — the read loop
/// tears it down; no shutdown needed.)
async fn send_frame_timed(
    writer: &Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    transport: &Arc<tokio::sync::Mutex<snow::TransportState>>,
    msg: &Message,
) -> bool {
    match tokio::time::timeout(STREAM_SEND_TIMEOUT, send_secure(writer, transport, msg)).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            tracing::warn!("Attachment stream send timed out; closing the stalled connection");
            let _ = writer.lock().await.shutdown().await;
            false
        }
    }
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
    if !send_frame_timed(
        &writer,
        &transport,
        &control_frame(MessageType::AttachmentFetchBegin, begin),
    )
    .await
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
        if !send_frame_timed(
            &writer,
            &transport,
            &control_frame(MessageType::AttachmentChunk, payload),
        )
        .await
        {
            return;
        }
    }

    let done = serde_json::to_string(&FetchDonePayload {
        attachment_id: attachment_id.to_string(),
        sha256: sha256.to_string(),
    })
    .unwrap_or_default();
    let _ = send_frame_timed(
        &writer,
        &transport,
        &control_frame(MessageType::AttachmentFetchDone, done),
    )
    .await;
}

/// Who is asking to reference attachment content. `Host` is the local trusted authority (it
/// already reads all plaintext and stores every blob directly), so it is exempt from the
/// possession/visibility gate. A remote peer is `Client(canonical_user_id)` and must prove it
/// may reference the content. This is a typed distinction (not a magic user id) so the
/// exemption can't be reached by a crafted frame.
pub enum RefActor {
    Host,
    Client(i64),
}

/// May `user_id` reference `sha256` in a message? Yes iff they hash-proved possession by
/// completing an upload (persistent `attachment_blob_uploaders` row) OR the content is already
/// visible to them in an accessible room (§9a item 9). Possession is checked first (cheap
/// indexed lookup by PK).
async fn may_reference_sha(pool: &SqlitePool, user_id: i64, sha256: &str) -> Result<bool, String> {
    if blob_store::user_uploaded(pool, sha256, user_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(true);
    }
    crate::db_queries::sha_referenced_in_accessible_room(pool, user_id, sha256).await
}

/// Validate the attachment refs on an inbound Chat frame BEFORE persisting or relaying
/// (§9a trap 1: the relay clones the whole message, so unvalidated refs would propagate).
/// Every ref must point at a complete stored blob whose size matches the claim, AND — for a
/// remote sender — content the sender is authorized to reference (§9a item 9). The host is
/// exempt from the reference gate (trusted authority) but still gets the structural checks.
pub async fn validate_chat_attachments(
    pool: &SqlitePool,
    actor: RefActor,
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
        // Reference authorization for a remote client, evaluated UNCONDITIONALLY (before the
        // existence/size check) so timing can't separate "stored but unauthorized" from
        // "absent": every ref that clears the field checks pays the same DB work regardless of
        // whether the blob exists. A genuine storage error is genericized to "Storage error"
        // (never raw sqlx text to the peer), matching the existence-check error arm below. Host
        // is the trusted authority → always authorized.
        let authorized = match actor {
            RefActor::Host => true,
            RefActor::Client(uid) => {
                may_reference_sha(pool, uid, &r.sha256).await.map_err(|e| {
                    tracing::error!("Attachment reference check failed: {}", e);
                    "Storage error".to_string()
                })?
            }
        };
        // Existence, size, AND authorization collapse into ONE rejection string, so an
        // unauthorized sender can't distinguish "exists but not yours" from "absent" by the
        // reply, timing, or message — a member who only learned a hash gains nothing.
        match blob_store::complete_blob_size(pool, &r.sha256).await {
            Ok(Some(stored)) if stored == r.size as i64 && authorized => {}
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

/// Upload `bytes` to the host over the live client connection and return the content sha.
/// One upload at a time; progress is emitted as `attachment_progress` events.
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

async fn await_signal(
    rx: &mut tokio::sync::mpsc::Receiver<UploadSignal>,
) -> Result<UploadSignal, String> {
    match tokio::time::timeout(CONTROL_REPLY_TIMEOUT, rx.recv()).await {
        Ok(Some(sig)) => Ok(sig),
        Ok(None) => Err("Connection lost during upload".to_string()),
        Err(_) => Err("Host did not respond — try again".to_string()),
    }
}

/// Request an attachment's bytes from the host (no-op if already cached or the same content
/// is already in flight — the `attachment_ready`/`attachment_failed` events carry the sha,
/// so the UI keys on it). The bytes land in the client cache; `get_attachment_bytes` reads
/// them out.
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
    if !register_pending_fetch(state, &attachment_id, &sha256).await? {
        return Ok(()); // already pending/in flight for this content
    }
    let payload = serde_json::to_string(&FetchPayload {
        attachment_id: attachment_id.clone(),
    })
    .map_err(|e| e.to_string())?;
    let sent =
        send_secure_client(state, &control_frame(MessageType::AttachmentFetch, payload)).await;
    if sent.is_err() {
        // Release the cap slot we reserved — the request never left.
        let mut pending = state.attachments_client.pending.lock().await;
        pending.remove(&attachment_id);
    }
    sent
}

/// Read a fetched/uploaded blob out of the client cache (`get_attachment_bytes`).
pub async fn cached_blob(state: &Arc<AppState>, sha256: &str) -> Option<Arc<Vec<u8>>> {
    let mut cache = state.attachments_client.cache.lock().await;
    cache.get(sha256)
}

/// Reset all client-side transfer state (disconnect/reconnect): fail the active upload,
/// drop pending fetches and in-flight assemblies. The cache survives — content-addressed
/// bytes stay valid. NOTE (§9a trap 12): dropped downloads emit no event from here; the UI
/// must fail in-flight fetch states on `connection_lost` (Phase 5 obligation).
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
    state.attachments_client.pending.lock().await.clear();
    state.attachments_client.downloads.lock().await.clear();
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
                accept_fetch_begin(state, &begin).await;
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
                // A failed fetch releases its pending cap slot (the host errored before or
                // instead of FetchBegin — e.g. denied, gone, or busy).
                if let Some(id) = &err.attachment_id {
                    let mut pending = state.attachments_client.pending.lock().await;
                    pending.remove(id);
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
        fail_download(
            app,
            state,
            &chunk.sha256,
            "Malformed attachment data — retry",
        )
        .await;
        return;
    }
    let Ok(bytes) = BASE64.decode(&chunk.data) else {
        fail_download(
            app,
            state,
            &chunk.sha256,
            "Malformed attachment data — retry",
        )
        .await;
        return;
    };

    let progress = {
        let mut downloads = state.attachments_client.downloads.lock().await;
        let Some(asm) = downloads.get_mut(&chunk.sha256) else {
            return; // unsolicited chunk — drop (no assembly to fail)
        };
        if chunk.seq != asm.next_seq || asm.buf.len() + bytes.len() > asm.expected {
            drop(downloads);
            fail_download(
                app,
                state,
                &chunk.sha256,
                "Attachment transfer error — retry",
            )
            .await;
            return;
        }
        asm.buf.extend_from_slice(&bytes);
        asm.next_seq += 1;
        (asm.buf.len(), asm.expected)
    };
    emit_progress(app, &chunk.sha256, "download", progress.0, progress.1);
}

/// Tear down an in-flight download and tell the UI, so a mid-stream protocol error surfaces
/// as a retryable failed card instead of a card that spins forever (§9a trap 12 family).
async fn fail_download(app: &tauri::AppHandle, state: &Arc<AppState>, sha256: &str, reason: &str) {
    let attachment_id = {
        let mut downloads = state.attachments_client.downloads.lock().await;
        downloads.remove(sha256).map(|asm| asm.attachment_id)
    };
    let Some(attachment_id) = attachment_id else {
        return; // nothing in flight for this sha — no card to fail
    };
    let _ = app.emit(
        "attachment_failed",
        serde_json::json!({
            "sha256": sha256,
            "attachment_id": attachment_id,
            "reason": reason,
        }),
    );
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

// ---- Tauri command boundary (design §9 task 4.1) ----
// One command each for upload / fetch / bytes / save, branching internally on host vs
// client mode (the host participant never touches the wire — its bytes live in its own DB).

/// What the composer needs to build an `AttachmentRef` after an upload: the content
/// address the store assigned, plus the size and display name taken from the file.
#[derive(Serialize)]
pub struct UploadedAttachment {
    pub sha256: String,
    pub size: u64,
    pub name: String,
}

/// Upload a file by path (from the attach dialog or a native drag-drop, both of which
/// yield paths — the path is user-chosen, never derived from message content). Host mode
/// stores straight into the blob store; client mode drives the wire upload.
///
/// TRUST BOUNDARY (reviewed, accepted for this threat model): this command reads whatever
/// absolute path the webview hands it. That is safe only because the webview is fully
/// trusted — strict CSP, no remote content, our own bundled JS, and no `dangerouslySetInnerHTML`
/// anywhere, so there is no script-injection point. The webview is *already* the full trust
/// boundary for every command (send-as-user, connect, etc.); this one extends a webview
/// compromise from "chat data" to "any readable file", which is why it is called out here.
/// If untrusted content rendering is ever introduced, move the file-picker path Rust-side
/// (open the dialog here, as `save_attachment` does) so the webview can't name arbitrary
/// paths — recorded as §9a item 14.
#[tauri::command(rename_all = "snake_case")]
pub async fn upload_attachment(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    path: String,
) -> Result<UploadedAttachment, String> {
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("Couldn't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("Only files can be attached".to_string());
    }
    // Early exit on the stat, but the authoritative cap is on the bytes actually read
    // below — a file can grow between stat and read (symlink swap, active writer), and
    // store_blob has no upper bound of its own.
    let too_big = |n: usize| n > MAX_ATTACHMENT_BYTES;
    if too_big(meta.len() as usize) {
        return Err(format!(
            "Files up to {} MB are supported",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Couldn't read the file: {e}"))?;
    if bytes.is_empty() {
        return Err("Cannot attach an empty file".to_string());
    }
    if too_big(bytes.len()) {
        return Err(format!(
            "Files up to {} MB are supported",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        ));
    }
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let size = bytes.len() as u64;

    let is_server = *state.is_server.read().await;
    let sha256 = if is_server {
        let sha = blob_store::store_blob(db.inner(), &bytes)
            .await
            .map_err(|e| e.to_string())?;
        // Mirror the client's instant-preview path: the host UI reads bytes back through
        // get_attachment_bytes, which serves the host from its DB — nothing else needed.
        emit_progress(&app, &sha, "upload", bytes.len(), bytes.len());
        sha
    } else {
        client_upload_attachment(&app, state.inner(), bytes).await?
    };
    Ok(UploadedAttachment { sha256, size, name })
}

/// Make an attachment's bytes available locally, firing `attachment_ready` (or
/// `attachment_failed`) with the sha when they are. Host mode verifies its own store;
/// client mode pulls over the wire (auto-fetch and click-to-download share this path).
#[tauri::command(rename_all = "snake_case")]
pub async fn fetch_attachment(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    attachment_id: String,
    sha256: String,
) -> Result<(), String> {
    let is_server = *state.is_server.read().await;
    if !is_server {
        return client_fetch_attachment(&app, state.inner(), attachment_id, sha256).await;
    }
    match blob_store::complete_blob_size(db.inner(), &sha256).await {
        Ok(Some(_)) => {
            let _ = app.emit(
                "attachment_ready",
                serde_json::json!({ "attachment_id": attachment_id, "sha256": sha256 }),
            );
            Ok(())
        }
        _ => {
            let _ = app.emit(
                "attachment_failed",
                serde_json::json!({
                    "sha256": sha256,
                    "attachment_id": attachment_id,
                    "reason": "Attachment is no longer available",
                }),
            );
            Ok(())
        }
    }
}

/// Hand an available blob's bytes to the webview as a raw IPC response (never JSON) —
/// the UI turns them into an object URL for previews. Host reads its store; client reads
/// its cache (populated by fetch_attachment / its own upload).
#[tauri::command(rename_all = "snake_case")]
pub async fn get_attachment_bytes(
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    sha256: String,
) -> Result<tauri::ipc::Response, String> {
    let bytes = attachment_bytes(state.inner(), db.inner(), &sha256).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Save an attachment to disk. The (sanitized) original filename is ONLY the dialog's
/// suggestion — the user-chosen dialog path is the single path we write (design §6).
/// Returns false when the user cancels the dialog.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_attachment(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    db: State<'_, SqlitePool>,
    sha256: String,
    suggested_name: String,
) -> Result<bool, String> {
    let bytes = attachment_bytes(state.inner(), db.inner(), &sha256).await?;
    let safe_name = crate::sanitize::sanitize_filename(&suggested_name);

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&safe_name)
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| "The save dialog closed unexpectedly".to_string())?
    else {
        return Ok(false); // user cancelled
    };
    let dest = picked.into_path().map_err(|e| e.to_string())?;
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("Couldn't save the file: {e}"))?;
    Ok(true)
}

/// Shared bytes lookup for the two read commands: host → blob store, client → cache.
async fn attachment_bytes(
    state: &Arc<AppState>,
    pool: &SqlitePool,
    sha256: &str,
) -> Result<Vec<u8>, String> {
    if !is_hex_sha256(sha256) {
        return Err("Invalid attachment hash".to_string());
    }
    if *state.is_server.read().await {
        blob_store::read_blob(pool, sha256)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Attachment is no longer available".to_string())
    } else {
        cached_blob(state, sha256)
            .await
            .map(|arc| arc.as_ref().clone())
            .ok_or_else(|| "Attachment isn't downloaded yet".to_string())
    }
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    use crate::db_queries::{
        create_room_internal, delete_message_db, insert_attachments_for_message,
        save_message_internal,
    };
    use crate::secure;
    use crate::sockets::{read_frame, ClientConnection};
    use snow::TransportState;
    use sqlx::sqlite::SqlitePoolOptions;

    const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            server_streams: Arc::new(tokio::sync::Mutex::new(Default::default())),
            client_stream: Arc::new(tokio::sync::Mutex::new(None)),
            client_transport: Arc::new(tokio::sync::Mutex::new(None)),
            client_listener: Arc::new(tokio::sync::Mutex::new(None)),
            client_heartbeat: Arc::new(tokio::sync::Mutex::new(None)),
            discovery_responder: Arc::new(tokio::sync::Mutex::new(None)),
            room_clients: Arc::new(tokio::sync::Mutex::new(Default::default())),
            ip_conn_counts: Arc::new(tokio::sync::Mutex::new(Default::default())),
            attachments_host: Default::default(),
            attachments_client: Default::default(),
            username: tokio::sync::RwLock::new(String::new()),
            user_id: tokio::sync::RwLock::new(None),
            is_server: tokio::sync::RwLock::new(false),
            current_room: tokio::sync::RwLock::new(String::new()),
            current_room_id: tokio::sync::RwLock::new(None),
            server_addr: tokio::sync::RwLock::new(None),
            pool: std::sync::OnceLock::new(),
            mdns: std::sync::Mutex::new(None),
        })
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory db");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        sqlx::raw_sql("PRAGMA foreign_keys=ON;")
            .execute(&pool)
            .await
            .expect("enable foreign keys");
        sqlx::raw_sql(
            "INSERT INTO users (id, name, email, department_id)
                 VALUES (1, 'Alice', 'a@x', 1), (2, 'Bob', 'b@x', 1);",
        )
        .execute(&pool)
        .await
        .expect("seed users");
        pool
    }

    /// The client's view of a registered connection: the host side lives in
    /// `state.server_streams[user_id]`; the returned halves keep the socket alive
    /// and let the test read the host's encrypted replies.
    struct TestClient {
        reader: tokio::net::tcp::OwnedReadHalf,
        transport: TransportState,
        // Held so the host-side read path never sees an EOF mid-test.
        _writer: tokio::net::tcp::OwnedWriteHalf,
        _host_reader: tokio::net::tcp::OwnedReadHalf,
    }

    /// Real socket pair + real Noise handshake, host side registered in server_streams
    /// under `user_id` — exactly the state the host handlers act on.
    async fn connect_client(state: &Arc<AppState>, user_id: u64) -> TestClient {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let psk = secure::derive_psk("transfer-tests");

        let (host_side, client_side) = tokio::join!(
            async {
                let (stream, _) = listener.accept().await.expect("accept");
                let (mut r, mut w) = stream.into_split();
                let t = secure::responder_handshake(&mut r, &mut w, &psk)
                    .await
                    .expect("responder handshake");
                (r, w, t)
            },
            async {
                let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
                let (mut r, mut w) = stream.into_split();
                let t = secure::initiator_handshake(&mut r, &mut w, &psk)
                    .await
                    .expect("initiator handshake");
                (r, w, t)
            }
        );
        let (host_r, host_w, host_t) = host_side;
        let (cli_r, cli_w, cli_t) = client_side;

        let conn = ClientConnection {
            writer: Arc::new(tokio::sync::Mutex::new(host_w)),
            transport: Arc::new(tokio::sync::Mutex::new(host_t)),
            username: format!("user-{user_id}"),
            current_room: "Company Wide".to_string(),
            room_id: 1,
            user_id,
            conn_id: user_id,
        };
        state.server_streams.lock().await.insert(user_id, conn);

        TestClient {
            reader: cli_r,
            transport: cli_t,
            _writer: cli_w,
            _host_reader: host_r,
        }
    }

    /// Read + decrypt the next host reply off the client socket (bounded, so a missing
    /// reply fails the test instead of hanging it).
    async fn next_reply(client: &mut TestClient) -> Message {
        let frame = tokio::time::timeout(REPLY_TIMEOUT, read_frame(&mut client.reader))
            .await
            .expect("timed out waiting for a host reply")
            .expect("read frame")
            .expect("non-empty frame");
        let plain = secure::decrypt(&mut client.transport, &frame).expect("decrypt reply");
        serde_json::from_str(std::str::from_utf8(&plain).expect("utf8"))
            .expect("reply deserializes as Message")
    }

    fn patterned_bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 249) as u8).collect()
    }

    fn start_payload(sha256: &str, size: usize) -> String {
        serde_json::to_string(&UploadStartPayload {
            sha256: sha256.to_string(),
            size,
        })
        .unwrap()
    }

    fn chunk_payload(sha256: &str, seq: u64, raw: &[u8]) -> String {
        serde_json::to_string(&ChunkPayload {
            sha256: sha256.to_string(),
            seq,
            data: BASE64.encode(raw),
        })
        .unwrap()
    }

    fn sha_payload(sha256: &str) -> String {
        serde_json::to_string(&ShaPayload {
            sha256: sha256.to_string(),
        })
        .unwrap()
    }

    fn fetch_payload(attachment_id: &str) -> String {
        serde_json::to_string(&FetchPayload {
            attachment_id: attachment_id.to_string(),
        })
        .unwrap()
    }

    /// Drive a complete wire upload of `bytes` for an already-connected `user_id`.
    async fn upload_via_wire(
        state: &Arc<AppState>,
        pool: &SqlitePool,
        client: &mut TestClient,
        user_id: u64,
        bytes: &[u8],
    ) -> String {
        let sha = blob_store::hex_sha256(bytes);
        handle_upload_start(state, pool, user_id, &start_payload(&sha, bytes.len())).await;
        let ready = next_reply(client).await;
        assert_eq!(ready.message_type, MessageType::AttachmentUploadReady);
        for (seq, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
            handle_upload_chunk(state, user_id, &chunk_payload(&sha, seq as u64, chunk)).await;
        }
        handle_upload_done(state, pool, user_id, &sha_payload(&sha)).await;
        let ok = next_reply(client).await;
        assert_eq!(ok.message_type, MessageType::AttachmentUploadOk);
        sha
    }

    /// Seed a Chat message row + attachment sidecar row pointing at `sha`, returning the
    /// attachment id a fetch would use.
    async fn seed_attachment_row(
        pool: &SqlitePool,
        room_id: i64,
        author: i64,
        message_id: &str,
        sha: &str,
        size: u64,
    ) -> String {
        save_message_internal(
            pool,
            room_id,
            author,
            "see attached".to_string(),
            "Chat".to_string(),
            false,
            message_id.to_string(),
        )
        .await
        .expect("save message row");
        let attachment_id = format!("att-{message_id}");
        insert_attachments_for_message(
            pool,
            message_id,
            &[AttachmentRef {
                id: attachment_id.clone(),
                sha256: sha.to_string(),
                name: "file.bin".to_string(),
                mime: "application/octet-stream".to_string(),
                size,
                width: None,
                height: None,
            }],
        )
        .await
        .expect("insert sidecar row");
        attachment_id
    }

    #[tokio::test]
    async fn upload_round_trips_and_fetch_streams_back_identical_bytes() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;
        let bytes = patterned_bytes(200 * 1024);

        let sha = upload_via_wire(&state, &pool, &mut client, 1, &bytes).await;
        assert!(blob_store::blob_exists_complete(&pool, &sha)
            .await
            .expect("exists check"));

        let att_id = seed_attachment_row(&pool, 1, 1, "msg-rt", &sha, bytes.len() as u64).await;
        handle_fetch(&state, &pool, 1, &fetch_payload(&att_id)).await;

        let begin = next_reply(&mut client).await;
        assert_eq!(begin.message_type, MessageType::AttachmentFetchBegin);
        let begin: FetchBeginPayload = serde_json::from_str(&begin.message).expect("begin json");
        assert_eq!(begin.sha256, sha);
        assert_eq!(begin.size, bytes.len());

        let mut assembled = Vec::with_capacity(bytes.len());
        let mut expect_seq = 0u64;
        loop {
            let frame = next_reply(&mut client).await;
            match frame.message_type {
                MessageType::AttachmentChunk => {
                    let chunk: ChunkPayload =
                        serde_json::from_str(&frame.message).expect("chunk json");
                    assert_eq!(chunk.sha256, sha);
                    assert_eq!(chunk.seq, expect_seq, "chunks must arrive in order");
                    expect_seq += 1;
                    assembled.extend_from_slice(&BASE64.decode(&chunk.data).expect("b64"));
                }
                MessageType::AttachmentFetchDone => break,
                other => panic!("unexpected frame during fetch: {other:?}"),
            }
        }
        assert_eq!(assembled, bytes);
        assert_eq!(blob_store::hex_sha256(&assembled), sha);
    }

    #[tokio::test]
    async fn oversize_upload_start_is_rejected_before_buffering() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;
        let sha = blob_store::hex_sha256(b"whatever");

        handle_upload_start(
            &state,
            &pool,
            1,
            &start_payload(&sha, MAX_ATTACHMENT_BYTES + 1),
        )
        .await;
        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);

        // No session was created: a chunk is silently dropped (no reply, no state)...
        handle_upload_chunk(&state, 1, &chunk_payload(&sha, 0, b"data")).await;
        assert!(state.attachments_host.uploads.lock().await.is_empty());

        // ...and a subsequent valid Start still works.
        handle_upload_start(&state, &pool, 1, &start_payload(&sha, 8)).await;
        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentUploadReady);
    }

    #[tokio::test]
    async fn upload_done_with_mismatched_hash_stores_nothing() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;

        let bytes = patterned_bytes(1024);
        let true_sha = blob_store::hex_sha256(&bytes);
        let declared_sha = blob_store::hex_sha256(b"something else entirely");

        handle_upload_start(&state, &pool, 1, &start_payload(&declared_sha, bytes.len())).await;
        assert_eq!(
            next_reply(&mut client).await.message_type,
            MessageType::AttachmentUploadReady
        );
        handle_upload_chunk(&state, 1, &chunk_payload(&declared_sha, 0, &bytes)).await;
        handle_upload_done(&state, &pool, 1, &sha_payload(&declared_sha)).await;

        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);
        assert!(!blob_store::blob_exists_complete(&pool, &declared_sha)
            .await
            .unwrap());
        assert!(!blob_store::blob_exists_complete(&pool, &true_sha)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn out_of_order_chunk_aborts_the_upload() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;
        let bytes = patterned_bytes(100 * 1024);
        let sha = blob_store::hex_sha256(&bytes);

        handle_upload_start(&state, &pool, 1, &start_payload(&sha, bytes.len())).await;
        assert_eq!(
            next_reply(&mut client).await.message_type,
            MessageType::AttachmentUploadReady
        );
        let chunks: Vec<&[u8]> = bytes.chunks(ATTACHMENT_CHUNK_BYTES).collect();
        handle_upload_chunk(&state, 1, &chunk_payload(&sha, 0, chunks[0])).await;
        handle_upload_chunk(&state, 1, &chunk_payload(&sha, 2, chunks[2])).await;

        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);
        assert!(state.attachments_host.uploads.lock().await.is_empty());

        // The connection can start over cleanly.
        handle_upload_start(&state, &pool, 1, &start_payload(&sha, bytes.len())).await;
        assert_eq!(
            next_reply(&mut client).await.message_type,
            MessageType::AttachmentUploadReady
        );
    }

    #[tokio::test]
    async fn fetch_is_denied_to_non_members() {
        let state = test_state();
        let pool = test_pool().await;
        let mut alice = connect_client(&state, 1).await;
        let mut bob = connect_client(&state, 2).await;

        // Alice creates a private room (creator auto-joins) and posts an attachment there.
        let room = create_room_internal(
            &pool,
            "alice-private".to_string(),
            None,
            None,
            Some(true),
            Some(1),
        )
        .await
        .expect("create private room");
        let room_id = room.id.expect("room id");

        let bytes = patterned_bytes(64 * 1024);
        let sha = upload_via_wire(&state, &pool, &mut alice, 1, &bytes).await;
        let att_id =
            seed_attachment_row(&pool, room_id, 1, "msg-priv", &sha, bytes.len() as u64).await;

        // Bob is not a member: error reply, and no FetchBegin ever reaches him.
        handle_fetch(&state, &pool, 2, &fetch_payload(&att_id)).await;
        let reply = next_reply(&mut bob).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);
        let err: ErrorPayload = serde_json::from_str(&reply.message).expect("error json");
        assert!(err.reason.contains("authorized"), "got: {}", err.reason);

        // Unknown attachment ids are indistinguishable from deleted ones.
        handle_fetch(&state, &pool, 2, &fetch_payload("no-such-attachment")).await;
        let reply = next_reply(&mut bob).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);
        let err: ErrorPayload = serde_json::from_str(&reply.message).expect("error json");
        assert!(
            err.reason.contains("no longer available"),
            "got: {}",
            err.reason
        );
    }

    #[tokio::test]
    async fn deleted_message_attachment_is_unfetchable() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;

        let bytes = patterned_bytes(32 * 1024);
        let sha = upload_via_wire(&state, &pool, &mut client, 1, &bytes).await;
        let att_id = seed_attachment_row(&pool, 1, 1, "msg-del", &sha, bytes.len() as u64).await;

        let deleted = delete_message_db(&pool, "msg-del", 1)
            .await
            .expect("delete");
        assert_eq!(deleted, 1);

        handle_fetch(&state, &pool, 1, &fetch_payload(&att_id)).await;
        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentError);
        let err: ErrorPayload = serde_json::from_str(&reply.message).expect("error json");
        assert!(
            err.reason.contains("no longer available"),
            "got: {}",
            err.reason
        );
    }

    #[tokio::test]
    async fn chat_frames_interleave_during_a_large_fetch() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;

        let bytes = patterned_bytes(4 * 1024 * 1024);
        let sha = blob_store::store_blob(&pool, &bytes).await.expect("store");
        let att_id = seed_attachment_row(&pool, 1, 1, "msg-big", &sha, bytes.len() as u64).await;

        handle_fetch(&state, &pool, 1, &fetch_payload(&att_id)).await;
        assert_eq!(
            next_reply(&mut client).await.message_type,
            MessageType::AttachmentFetchBegin
        );

        // While the chunk stream is flowing, race a Chat frame onto the same connection.
        let conn = {
            let streams = state.server_streams.lock().await;
            let c = streams.get(&1).expect("registered conn");
            (Arc::clone(&c.writer), Arc::clone(&c.transport))
        };
        let mut chat = control_frame(MessageType::Chat, "interleaved hello".to_string());
        chat.room = "Company Wide".to_string();
        chat.room_id = 1;
        let sender = tauri::async_runtime::spawn(async move {
            send_secure(&conn.0, &conn.1, &chat)
                .await
                .expect("chat send");
        });

        let mut saw_chat_at: Option<usize> = None;
        let mut frames = 0usize;
        loop {
            let frame = next_reply(&mut client).await;
            frames += 1;
            match frame.message_type {
                MessageType::Chat => saw_chat_at = Some(frames),
                MessageType::AttachmentFetchDone => break,
                MessageType::AttachmentChunk => {}
                other => panic!("unexpected frame: {other:?}"),
            }
        }
        sender.await.expect("sender task");
        let position = saw_chat_at.expect("chat frame must arrive during the stream");
        assert!(
            position < frames,
            "chat frame arrived at {position}, only with/after FetchDone at {frames} — head-of-line blocked"
        );
    }

    #[tokio::test]
    async fn duplicate_upload_start_after_stored_blob_fast_paths_to_ok() {
        let state = test_state();
        let pool = test_pool().await;
        let mut client = connect_client(&state, 1).await;
        let bytes = patterned_bytes(50 * 1024);

        let sha = upload_via_wire(&state, &pool, &mut client, 1, &bytes).await;

        // Same content again: immediate UploadOk, no Ready, no session created.
        handle_upload_start(&state, &pool, 1, &start_payload(&sha, bytes.len())).await;
        let reply = next_reply(&mut client).await;
        assert_eq!(reply.message_type, MessageType::AttachmentUploadOk);
        assert!(state.attachments_host.uploads.lock().await.is_empty());
    }

    #[tokio::test]
    async fn validate_chat_attachments_rejects_bad_refs() {
        let pool = test_pool().await;
        let bytes = patterned_bytes(10 * 1024);
        let sha = blob_store::store_blob(&pool, &bytes).await.expect("store");

        let make_ref = |id: &str, sha: &str, size: u64| AttachmentRef {
            id: id.to_string(),
            sha256: sha.to_string(),
            name: "f.bin".to_string(),
            mime: "application/octet-stream".to_string(),
            size,
            width: None,
            height: None,
        };

        // A valid single ref is accepted.
        let valid = make_ref("a1", &sha, bytes.len() as u64);
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, std::slice::from_ref(&valid))
                .await
                .is_ok()
        );

        // Too many refs.
        let many: Vec<_> = (0..=MAX_ATTACHMENTS_PER_MESSAGE)
            .map(|i| make_ref(&format!("m{i}"), &sha, bytes.len() as u64))
            .collect();
        assert!(validate_chat_attachments(&pool, RefActor::Host, &many)
            .await
            .is_err());

        // Size mismatch vs the stored blob.
        let wrong_size = make_ref("a2", &sha, 1);
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, &[wrong_size])
                .await
                .is_err()
        );

        // Never-uploaded content.
        let ghost_sha = blob_store::hex_sha256(b"never uploaded");
        let ghost = make_ref("a3", &ghost_sha, 42);
        assert!(validate_chat_attachments(&pool, RefActor::Host, &[ghost])
            .await
            .is_err());

        // Duplicate ref ids within one message.
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, &[valid.clone(), valid])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fetch_begin_is_only_honored_for_requested_fetches() {
        let state = test_state();
        let sha = blob_store::hex_sha256(b"content");

        // Unsolicited FetchBegin: rejected, no assembly allocated (§9a trap 10).
        let unsolicited = FetchBeginPayload {
            attachment_id: "att-x".to_string(),
            sha256: sha.clone(),
            size: 1024,
        };
        assert!(!accept_fetch_begin(&state, &unsolicited).await);
        assert!(state.attachments_client.downloads.lock().await.is_empty());

        // Requested fetch: accepted once, pending slot consumed.
        assert!(register_pending_fetch(&state, "att-x", &sha).await.unwrap());
        assert!(accept_fetch_begin(&state, &unsolicited).await);
        assert!(state
            .attachments_client
            .downloads
            .lock()
            .await
            .contains_key(&sha));
        // A repeat Begin for the same content is unsolicited again AND must not clobber
        // the in-flight assembly.
        assert!(!accept_fetch_begin(&state, &unsolicited).await);

        // A sha-swapped Begin for a pending id is rejected AND releases the slot (a host
        // can't substitute content, nor pin the slot by answering with a wrong sha).
        let other_sha = blob_store::hex_sha256(b"other");
        assert!(register_pending_fetch(&state, "att-y", &other_sha)
            .await
            .unwrap());
        let swapped = FetchBeginPayload {
            attachment_id: "att-y".to_string(),
            sha256: sha.clone(),
            size: 1024,
        };
        assert!(!accept_fetch_begin(&state, &swapped).await);
        assert!(!state
            .attachments_client
            .pending
            .lock()
            .await
            .contains_key("att-y"));

        // An oversize Begin consumes the pending slot but allocates nothing.
        assert!(register_pending_fetch(&state, "att-z", &sha).await.is_ok());
        // (att-z shares sha with an in-flight download, so register dedupes it — use a
        // fresh sha to exercise the size gate.)
        let big_sha = blob_store::hex_sha256(b"big");
        assert!(register_pending_fetch(&state, "att-big", &big_sha)
            .await
            .unwrap());
        let oversize = FetchBeginPayload {
            attachment_id: "att-big".to_string(),
            sha256: big_sha.clone(),
            size: MAX_ATTACHMENT_BYTES + 1,
        };
        assert!(!accept_fetch_begin(&state, &oversize).await);
        assert!(!state
            .attachments_client
            .downloads
            .lock()
            .await
            .contains_key(&big_sha));
        assert!(!state
            .attachments_client
            .pending
            .lock()
            .await
            .contains_key("att-big"));
    }

    #[tokio::test]
    async fn pending_fetch_registry_dedupes_and_caps() {
        let state = test_state();

        // Same id or same content dedupes to Ok(false) — one wire fetch per content.
        let sha0 = blob_store::hex_sha256(b"c0");
        assert!(register_pending_fetch(&state, "a0", &sha0).await.unwrap());
        assert!(!register_pending_fetch(&state, "a0", &sha0).await.unwrap());
        assert!(!register_pending_fetch(&state, "a0-dup", &sha0)
            .await
            .unwrap());

        // Fill to the cap with distinct content, then the next registration errors.
        for i in 1..MAX_CLIENT_ASSEMBLIES {
            let sha = blob_store::hex_sha256(format!("c{i}").as_bytes());
            assert!(register_pending_fetch(&state, &format!("a{i}"), &sha)
                .await
                .unwrap());
        }
        let overflow_sha = blob_store::hex_sha256(b"overflow");
        assert!(register_pending_fetch(&state, "a-overflow", &overflow_sha)
            .await
            .is_err());

        // reset_client releases every slot.
        reset_client(&state).await;
        assert!(register_pending_fetch(&state, "a-overflow", &overflow_sha)
            .await
            .unwrap());
    }

    /// Two connections uploading identical bytes concurrently must both end in a
    /// non-broken state with the blob stored exactly once. This needs a real
    /// multi-connection pool (an `sqlite::memory:` pool gives each connection its own
    /// DB), configured like production (WAL + busy_timeout), so the same-sha
    /// DELETE+INSERT transactions actually contend on SQLite's write lock.
    #[tokio::test]
    async fn concurrent_duplicate_uploads_store_once_and_both_succeed() {
        let mut path = std::env::temp_dir();
        path.push(format!("nutler-transfer-test-{}.db", Uuid::new_v4()));
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .expect("open temp-file db");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        sqlx::raw_sql(
            "INSERT INTO users (id, name, email, department_id)
                 VALUES (1, 'Alice', 'a@x', 1), (2, 'Bob', 'b@x', 1);",
        )
        .execute(&pool)
        .await
        .expect("seed users");

        let state = test_state();
        let mut alice = connect_client(&state, 1).await;
        let mut bob = connect_client(&state, 2).await;
        let bytes = patterned_bytes(300 * 1024);
        let sha = blob_store::hex_sha256(&bytes);

        // Drive one full wire upload; tolerate the dedup fast-path (UploadOk straight
        // from Start) since the peer may have completed first.
        async fn upload_racy(
            state: &Arc<AppState>,
            pool: &SqlitePool,
            client: &mut TestClient,
            user_id: u64,
            bytes: &[u8],
            sha: &str,
        ) {
            handle_upload_start(state, pool, user_id, &start_payload(sha, bytes.len())).await;
            let reply = next_reply(client).await;
            match reply.message_type {
                MessageType::AttachmentUploadOk => return, // dedup fast-path
                MessageType::AttachmentUploadReady => {}
                other => panic!("unexpected reply to Start: {other:?}"),
            }
            for (seq, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
                handle_upload_chunk(state, user_id, &chunk_payload(sha, seq as u64, chunk)).await;
            }
            handle_upload_done(state, pool, user_id, &sha_payload(sha)).await;
            let done = next_reply(client).await;
            assert_eq!(
                done.message_type,
                MessageType::AttachmentUploadOk,
                "a byte-perfect upload must not surface a storage error to its sender"
            );
        }

        tokio::join!(
            upload_racy(&state, &pool, &mut alice, 1, &bytes, &sha),
            upload_racy(&state, &pool, &mut bob, 2, &bytes, &sha),
        );

        // Exactly one blob row + one coherent chunk set survived the race.
        let blob_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blobs WHERE sha256 = $1")
                .bind(&sha)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(blob_rows, 1);
        assert!(blob_store::blob_exists_complete(&pool, &sha).await.unwrap());
        let read = blob_store::read_blob(&pool, &sha)
            .await
            .unwrap()
            .expect("blob readable");
        assert_eq!(read, bytes);

        pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn byte_rate_limiter_refills_over_time() {
        let mut limiter = ByteRateLimiter::new(tokio::time::Instant::now());

        // The full burst is available up front...
        assert!(limiter.allow(tokio::time::Instant::now(), CHUNK_BURST_BYTES as usize));
        // ...then the bucket is empty.
        assert!(!limiter.allow(tokio::time::Instant::now(), 1024));

        // One second refills ~CHUNK_RATE_BYTES_PER_SEC.
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(limiter.allow(
            tokio::time::Instant::now(),
            (CHUNK_RATE_BYTES_PER_SEC as usize) - 1024
        ));
        assert!(!limiter.allow(
            tokio::time::Instant::now(),
            CHUNK_RATE_BYTES_PER_SEC as usize
        ));
    }

    #[tokio::test]
    async fn client_cannot_reference_unowned_sha() {
        let pool = test_pool().await; // users 1, 2
        let bytes = patterned_bytes(4096);
        let sha = blob_store::store_blob(&pool, &bytes).await.unwrap();
        // User 1 uploaded it; it lives only in user 1's PRIVATE room (user 2 not a member).
        blob_store::record_uploader(&pool, &sha, 1).await.unwrap();
        let room = create_room_internal(&pool, "priv".to_string(), None, None, Some(true), Some(1))
            .await
            .unwrap();
        let rid = room.id.unwrap();
        let _ = seed_attachment_row(&pool, rid, 1, "msg-unowned", &sha, bytes.len() as u64).await;

        let r = AttachmentRef {
            id: "new-att".to_string(),
            sha256: sha.clone(),
            name: "f.bin".to_string(),
            mime: "application/octet-stream".to_string(),
            size: bytes.len() as u64,
            width: None,
            height: None,
        };
        // User 2 knows the hash but never uploaded it and can't see the room → rejected, with
        // the SAME string as "not uploaded" (no exists-but-not-yours oracle).
        let err = validate_chat_attachments(&pool, RefActor::Client(2), std::slice::from_ref(&r))
            .await
            .unwrap_err();
        assert_eq!(err, "Attachment was not uploaded");
        // User 1 (the uploader) may reference it.
        assert!(
            validate_chat_attachments(&pool, RefActor::Client(1), std::slice::from_ref(&r))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn uploader_may_reference_after_recording() {
        let pool = test_pool().await;
        let bytes = patterned_bytes(2048);
        let sha = blob_store::store_blob(&pool, &bytes).await.unwrap();
        let r = AttachmentRef {
            id: "a".to_string(),
            sha256: sha.clone(),
            name: "f.bin".to_string(),
            mime: "application/octet-stream".to_string(),
            size: bytes.len() as u64,
            width: None,
            height: None,
        };
        // No possession, no room visibility → rejected.
        assert!(
            validate_chat_attachments(&pool, RefActor::Client(2), std::slice::from_ref(&r))
                .await
                .is_err()
        );
        // After recording possession (survives reconnect — it's a persistent row) → allowed.
        blob_store::record_uploader(&pool, &sha, 2).await.unwrap();
        assert!(
            validate_chat_attachments(&pool, RefActor::Client(2), std::slice::from_ref(&r))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn host_actor_exempt_but_still_structural() {
        let pool = test_pool().await;
        let bytes = patterned_bytes(1024);
        let sha = blob_store::store_blob(&pool, &bytes).await.unwrap();
        let r = AttachmentRef {
            id: "a".to_string(),
            sha256: sha.clone(),
            name: "f.bin".to_string(),
            mime: "application/octet-stream".to_string(),
            size: bytes.len() as u64,
            width: None,
            height: None,
        };
        // Host: no uploader row, no room visibility → still Ok (trusted authority, exempt).
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, std::slice::from_ref(&r))
                .await
                .is_ok()
        );
        // Structural checks still apply to the host: size mismatch and non-existent sha fail.
        let bad_size = AttachmentRef {
            size: 999_999,
            ..r.clone()
        };
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, std::slice::from_ref(&bad_size))
                .await
                .is_err()
        );
        let ghost = AttachmentRef {
            sha256: blob_store::hex_sha256(b"never stored"),
            ..r.clone()
        };
        assert!(
            validate_chat_attachments(&pool, RefActor::Host, std::slice::from_ref(&ghost))
                .await
                .is_err()
        );
    }
}
