# Nutler Attachments — Design & Implementation Plan

**Status:** Approved (2026-07-02) — all §10 judgment calls signed off; storage must sit behind a replaceable abstraction (see §3a)
**Grounding:** `src-tauri/src/sockets.rs`, `secure.rs`, `migration.rs`, `db_queries.rs`, `db.rs`, `src/types.ts`, `src/hooks/`, [messaging-platform.md](./messaging-platform.md) Constraint 8, [gap-analysis.md](./gap-analysis.md) near-term move #2

---

## 0. One correction to the framing (load-bearing)

**The 10 MB `MAX_FRAME_BYTES` is dead headroom.** `send_secure` (sockets.rs) serializes the whole `Message` JSON and encrypts it as **one** Noise record; `secure::encrypt` hard-rejects plaintext > 65,519 bytes. There is **no** transport-layer chunking of large frames today — an oversize send simply errors. The effective per-message wire cap is ~64 KB, so any blob design must chunk at the **application** layer at ≤ ~64 KB per frame. This also means head-of-line blocking is bounded per chunk (~64 KB, sub-millisecond on LAN), not per blob — which is what makes "reuse the existing connection" viable.

Second correction: "eager push for small images" should not be a host-side push path. It is implemented as **client-initiated auto-fetch** — same pull path, the client just chooses to pull immediately for small images. One host code path, one membership gate.

---

## 1. Envelope shape — optional field, no version bump, capability-gated

**Decision.** Add an optional, serde-defaulted `attachments` field to the wire `Message`. Keep `PROTOCOL_VERSION = 1`. Gate the feature on a host capability advertisement rather than a version bump.

Rust (`sockets.rs`):

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct AttachmentRef {
    pub id: String,          // client-generated UUID (per message-attachment instance)
    pub sha256: String,      // hex content address of the blob
    pub name: String,        // original filename (data, never a path)
    pub mime: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,  // images only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

// on Message:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub attachments: Option<Vec<AttachmentRef>>,

// on Message (capability negotiation, set by the host on the Identity frame):
#[serde(default, skip_serializing_if = "Option::is_none")]
pub features: Option<Vec<String>>,   // e.g. ["attachments-v1"]
```

TypeScript (`src/types.ts`):

```ts
export interface AttachmentRef {
  id: string; sha256: string; name: string; mime: string; size: number;
  width?: number; height?: number;
}
// Message gains: attachments?: AttachmentRef[];
```

**Rationale.**
- The north-star envelope (Constraint 7) is literally `{version, metadata, payload, attachments}` — attachments as their own separable content slot **is** the gap-analysis move-#2 shape. `AttachmentRef` is the unit that later becomes ciphertext (filename/mime are content, not routing metadata); the change stays localized.
- No `PROTOCOL_VERSION = 2`: old peers decode the frame fine (serde_json ignores unknown fields), and version bumps should be reserved for changes where a peer must *branch or reject* (e.g. ciphertext payloads). An old client receiving a Chat-with-attachments sees the caption text and no attachment — graceful degradation.
- **The real coexistence hazard is new `MessageType` variants, not the field.** An old *host* receiving an unknown variant fails `serde_json::from_str` and **drops the connection** (`handle_client_connection`). So a new client must never send attachment frames to an old host. Hence `features` on the `Identity` frame: new hosts advertise `attachments-v1`; the client enables the attach UI only when it sees the flag (host-mode UI always enables — the host is local). Old hosts never advertise → attach button disabled with a "host doesn't support attachments" tooltip. Old clients never receive attachment frames because the host only sends chunks in response to an explicit fetch.

---

## 2. Blob transfer — chunked over the existing connection, upload-then-reference, pull-based download

**Decision.** Reuse the single framed Noise TCP connection. No second channel.

- A separate blob channel means a second Noise handshake, a second session to authenticate/track/tear down, and duplicated auth logic — the exact session/auth surface we should not multiply for a v1. With ~64 KB chunks, chat frames interleave between chunk sends (each chunk send acquires and releases the writer mutex), so HOL blocking is bounded at one chunk.
- **Backpressure** is natural: the host streams a download sequentially, awaiting each `write_all`; TCP flow control + the writer mutex pace it.

**Constants:**

| Constant | Value | Why |
|---|---|---|
| `MAX_ATTACHMENT_BYTES` | **25 MiB** | ~580 chunks ≈ a few seconds on LAN; bounds host memory and DB growth |
| `ATTACHMENT_CHUNK_BYTES` | **44 KiB raw** | base64(45,056) ≈ 60,076 B + JSON wrapper + envelope < 65,519 Noise cap with headroom |
| `MAX_ATTACHMENTS_PER_MESSAGE` | 5 | UI + frame-size sanity |
| Per-connection concurrent transfers | 1 upload, 2 downloads | bounds memory and write-path contention |
| Host-wide concurrent uploads | 4 | worst-case buffering 100 MiB |

**Flow (two-phase: blob first, then message):**

1. Client → host `AttachmentUploadStart` — payload `{sha256, size}`. Host validates: size ≤ cap, sender authenticated, upload slot free. If the blob already exists complete (dedup), reply `AttachmentUploadOk` immediately — no transfer.
2. Host → client `AttachmentUploadReady`.
3. Client → host `AttachmentChunk` × N — payload `{sha256, seq, data}` (base64). Host enforces sequential `seq`, running byte count ≤ declared size, buffers in memory (≤ 25 MiB).
4. Client → host `AttachmentUploadDone {sha256}` — host hashes the assembled bytes, verifies against the declared sha256, persists blob + chunk rows in one transaction, replies `AttachmentUploadOk {sha256}` (or `AttachmentError {reason}` and discards).
5. Client sends the normal `Chat` message with `attachments: [AttachmentRef{...}]`. The host, on saving the Chat (after the existing membership gate), inserts the `attachments` sidecar rows.
6. Recipients render the message from metadata alone. To get bytes: client → host `AttachmentFetch {attachment_id}` → host gates (see §6) → `AttachmentFetchBegin {attachment_id, sha256, size}` → `AttachmentChunk` stream → `AttachmentFetchDone {attachment_id}`. Client verifies sha256 on receipt.

**New `MessageType` variants (exact):** `AttachmentUploadStart`, `AttachmentUploadReady`, `AttachmentChunk`, `AttachmentUploadDone`, `AttachmentUploadOk`, `AttachmentFetch`, `AttachmentFetchBegin`, `AttachmentFetchDone`, `AttachmentError`. Structured payloads ride in the existing `message: String` field as JSON — consistent with how `HistoryRequest`/`RoomCreate`/`DmRequest` already work; no new envelope fields beyond §1.

**Rate limiter interaction.** The existing token bucket (10 msg/s, burst 20) would throttle a 25 MiB upload to ~1 minute and starve chat. Change: `AttachmentChunk` frames bypass the message-count bucket and are governed by a **separate per-connection byte bucket** (e.g. 4 MiB/s sustained, 8 MiB burst, counted on decoded bytes). All other attachment control frames stay in the normal bucket. Chunks arriving with no accepted upload in progress are dropped and count against the byte bucket (flood-safe).

**Host-as-sender / host-as-viewer:** the host participant never touches the wire — a Tauri command stores the blob directly in its DB and distributes the Chat; viewing reads the DB directly. This mirrors the existing `send_as_server_participant` / `send_as_client` split.

---

## 3. Storage — SQLite blob-chunk rows inside the existing SQLCipher DB

**Decision.** Store blobs as chunked BLOB rows in the host's SQLCipher database. No filesystem blob dir.

**Rationale.**
- **At-rest posture for free.** The DB is SQLCipher-encrypted (db.rs); files under app-data would be plaintext — a regression that would then demand a hand-rolled per-file encryption layer (new key handling in security-critical code, exactly what we shouldn't add casually).
- **No filesystem attack surface.** No filenames ever touch the disk as paths — the path-traversal class disappears at the storage layer entirely.
- **Transactional GC** with message delete; no DB↔filesystem consistency dance.
- Cost: DB growth and WAL churn. At a 25 MiB cap on a LAN team tool this is acceptable; chunked rows (256 KiB) keep reads/writes streaming-friendly. If usage outgrows this, migrating to encrypted files is a storage-layer swap behind the same queries — flagged as a known revisit, not built now (Guardrail 1).

### 3a. Storage abstraction (maintainer requirement)

The rest of the application deals only in attachment IDs / `AttachmentRef`s — nothing above the storage layer may know the bytes live in SQLCipher. Implementation: a dedicated `blob_store.rs` module owning all blob persistence (`store_blob`, `blob_exists`, `read_blob` streamed, `delete_orphans`), keyed by sha256. The wire handlers, Tauri commands, and GC call only this surface. SQLCipher chunk-rows are today's backend; swapping to encrypted files or object storage must touch only this module. (Per Guardrail 1 this is a module seam, not a trait with a single impl — promote to a trait when a second backend is real.)

**Content addressing:** yes — blobs keyed by sha256 with per-message `attachments` rows referencing them. Re-sending the same file stores once and skips the upload (dedup at `AttachmentUploadStart`). Note: this is plaintext-hash dedup and is E2EE-incompatible cross-user (see §8).

**Client-side cache: in-memory, session-scoped (v1).** Fetched blobs live in a Rust-side `HashMap<sha256, Bytes>` with an LRU byte cap (e.g. 256 MiB). No client DB rows, no disk cache — re-fetching on a LAN is cheap, and it keeps the client/host DB split clean (clients keep no host data, per the RoomList/History pattern). Persistent client cache is a later enhancement. Blob bytes reach the frontend via a Tauri command returning raw bytes (`tauri::ipc::Response`), never base64-through-JSON events; saving to disk is done Rust-side (dialog-chosen path), so 25 MiB never crosses the IPC boundary as JSON.

---

## 4. Migration v14 sketch

```sql
-- v14: attachment metadata sidecar + content-addressed encrypted-at-rest blob store.
-- Blob bytes are OPAQUE to all queries: nothing may parse them (E2EE forward-compat).
CREATE TABLE attachment_blobs (
    sha256     TEXT PRIMARY KEY,
    size       INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE attachment_blob_chunks (
    sha256 TEXT NOT NULL REFERENCES attachment_blobs(sha256) ON DELETE CASCADE,
    seq    INTEGER NOT NULL,
    data   BLOB NOT NULL,
    PRIMARY KEY (sha256, seq)
);
CREATE TABLE attachments (
    id         TEXT PRIMARY KEY,        -- client-generated UUID (AttachmentRef.id)
    message_id TEXT NOT NULL,           -- messages.message_id (soft-deleted msgs GC'd in code)
    sha256     TEXT NOT NULL REFERENCES attachment_blobs(sha256),
    filename   TEXT NOT NULL,
    mime       TEXT NOT NULL,
    size       INTEGER NOT NULL,
    width      INTEGER,
    height     INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_attachments_message ON attachments(message_id);
CREATE INDEX idx_attachments_sha ON attachments(sha256);
```

Notes: no FK from `attachments.message_id` to `messages` — message deletion is a soft delete (`deleted_at`), so attachment/blob GC is explicit code (§5), and we avoid insert-ordering coupling. Include a `Down` migration. The schema ships to all peers (one binary); clients simply never populate it in v1.

---

## 5. Lifecycle

- **Send:** composer picks file(s) → upload phase with progress (per §2) → on `AttachmentUploadOk`, send Chat with `attachments` → optimistic render using the local bytes (seed the client cache so the sender's own image previews instantly).
- **Receive (live):** Chat arrives with metadata → render card/placeholder immediately. Images with `size ≤ 512 KiB` auto-fetch (client-initiated); everything else fetches on click.
- **History:** `send_room_history` already builds `{messages, reactions}` and trims to one Noise frame. Add a third key, `attachments`, filtered to the surviving message ids — exactly the reactions pattern. **Metadata only, ~150 bytes per attachment; blob bytes never enter history frames**, so the existing trim loop keeps its invariant. `get_room_messages_internal` callers get attachments via one extra query keyed by the page's message_ids.
- **Delete:** in the `MessageType::Delete` handler, after `delete_message_db` succeeds: `DELETE FROM attachments WHERE message_id = ?;` then `DELETE FROM attachment_blobs WHERE sha256 NOT IN (SELECT DISTINCT sha256 FROM attachments);` (chunks cascade). Fetch of a deleted message's attachment fails the gate (row gone). Edit does not touch attachments in v1.
- **Orphan sweep:** uploaded blobs whose Chat never arrived (client crashed between phases) are GC'd by a sweep at host startup + hourly: complete blobs with zero `attachments` rows older than 1 hour.

---

## 6. Security

| Concern | Enforcement |
|---|---|
| Allocation before validation | `AttachmentUploadStart` declares size; rejected > 25 MiB **before** any buffering. Running byte count per chunk; exceeding declared size aborts and discards. Chunk base64 length capped. Sequential `seq` required. |
| Rate/flood | Chunk byte-bucket per connection (§2); 1 upload + 2 downloads per connection; 4 uploads host-wide; unsolicited chunks dropped. |
| Who may fetch | `AttachmentFetch` resolves `attachment_id → message_id → room_id`, then the **same** `room_join_allowed_internal` gate as `HistoryRequest`/`Chat`, keyed on the connection's canonical `user_id` (never the frame's). Fetch by attachment id, not by sha256, so knowing a hash grants nothing. Uploads require an authenticated connection (post-`Connect`). |
| Filename handling | Filename is stored and displayed as data only. It is never used to construct a path. On save, it is only the *suggested* name for the tauri-plugin-dialog save dialog (sanitized: strip `/ \ ..` and control chars, cap length); the user-chosen dialog path is what Rust writes to. |
| Integrity | Host verifies sha256 of assembled upload before persisting; client verifies sha256 of assembled download before caching/rendering. Mismatch → discard + `AttachmentError` / UI failure state. |
| Content parsing | The host never inspects blob bytes — no sniffing, no thumbnailing, no indexing. MIME is sender-asserted display metadata. Rendering images from untrusted bytes happens in the recipient's webview via blob URL (same trust level as today's untrusted text). |

---

## 7. UI/UX sketch (brief)

- **Composer:** paperclip button (file dialog via `@tauri-apps/plugin-dialog`) + drag-drop onto `ChatPane` + paste-image. Selected files show as removable chips with name/size; over-cap files rejected inline ("Files up to 25 MB"). Disabled (with tooltip) when the host lacks `attachments-v1`.
- **Message rendering:** images → inline preview (constrained box using `width`/`height` for layout stability, click to open full size); other files → card with icon, name, human size, and a Download action.
- **States:** uploading (progress bar on the optimistic message, cancel), fetching (spinner/progress on the card), failed (retry affordance + reason), sha-mismatch ("file corrupted in transfer — retry").
- **Progress plumbing:** Rust emits `attachment_progress {sha256, direction, transferred, total}` events; a `useAttachments` hook owns per-sha transfer state and the fetched-object-URL map (revoking URLs on unmount).

---

## 8. Flagged for gap-analysis (E2EE-incompatible / metadata-visible)

Add to the E2EE-incompatible table in [gap-analysis.md](./gap-analysis.md):

1. **Host stores attachment blobs plaintext-readable** (SQLCipher protects at rest, but the host process reads content). Future path: sender encrypts the blob per-message-key; host stores ciphertext under the same schema — the storage layer is already opaque-bytes, so this is localized.
2. **Plaintext-content sha256 dedup**: identical files dedup across users, meaning the host learns content equality. Under E2EE, hashes are of ciphertext and cross-user dedup disappears (accepted).
3. **Attachment metadata (filename, mime, size, dimensions) is host-visible** in the sidecar and in history frames. Under E2EE, filename/mime move into the encrypted content; size remains visible (Constraint 6 table already accepts size leakage).

Deliberately **not** introduced: server-side thumbnailing, content indexing, mime-based server behavior, previews — thumbnails, if wanted later, are client-generated second blobs (noted in the design, not built).

Also add **ADR-0005** ("Attachments: chunked transfer over the existing Noise connection; content-addressed SQLCipher blob store; capability-gated, no protocol version bump") in the decisions.md format.

---

## 9. Phased task list

Every task self-contained; sizes S/M/L; **BULK** = delegable to Codex, **TASTE** = Claude models. Per review policy, everything touching `sockets.rs`/wire protocol, migrations, Tauri command contracts, or chat UI is TASTE regardless of apparent simplicity.

### Phase 1 — Envelope + schema foundations
| # | Task | Mark | Size |
|---|---|---|---|
| 1.1 | Add `AttachmentRef` struct, `Message.attachments`, `Message.features` to `sockets.rs`; set `features: ["attachments-v1"]` on the `Identity` frame; mirror `AttachmentRef` + `Message.attachments` in `src/types.ts` and thread through `normalizeMessage` in `useChatConnection.ts`. | **TASTE** (wire + TS contract) | S |
| 1.2 | Migration v14 (schema in §4) + Down, bump the migration test's latest-version assertion in `db.rs`. | **TASTE** (migration.rs) | S |
| 1.3 | Serde golden tests: old-frame JSON (no attachments/features) decodes; new frame with attachments round-trips; frame with *unknown extra fields* decodes (guards the coexistence claim). | **BULK** | S |

Verify: `cargo test` in `src-tauri/` + `npm run typecheck`.

### Phase 2 — Host blob store (DB layer)
| # | Task | Mark | Size |
|---|---|---|---|
| 2.1 | New `blob_store.rs` module (the §3a seam): `store_blob` (transactional chunk insert), `blob_exists_complete`, `read_blob` (streamed by seq), `delete_orphan_blobs`; plus `db_queries.rs` metadata queries `insert_attachments_for_message`, `get_attachments_for_message_ids`, `delete_attachments_for_message`. Unit tests for each incl. GC refcount behavior. | **BULK** (clear spec, pure SQL/Rust; escalate review since host-DB-adjacent) | M |
| 2.2 | Filename-sanitizer utility (Rust) + tests (path separators, `..`, control chars, length cap, unicode). | **BULK** | S |

Verify: `cargo test`.

### Phase 3 — Wire transfer (host + client Rust)
| # | Task | Mark | Size |
|---|---|---|---|
| 3.1 | New `MessageType` variants + host-side handlers in `handle_server_message`/read loop: upload state machine (Start/Ready/Chunk/Done/Ok/Error), size + seq + concurrency caps, chunk byte-bucket rate limiter (chunks exempt from the count bucket), sha verification, persist via 2.1. Membership-gated `AttachmentFetch` → `FetchBegin`/chunk stream/`FetchDone` with sequential paced writes. | **TASTE** (sockets.rs, protocol core) | L |
| 3.2 | Client-side transfer engine: upload driver (chunker, progress events, awaiting Ok), download assembler (sha verify, LRU in-memory blob cache), `attachment_progress`/`attachment_ready` events. | **TASTE** (sockets.rs client path) | M |
| 3.3 | Loopback integration tests: full upload→Chat→fetch round trip over a real socket pair; oversize rejected pre-buffer; wrong sha discarded; non-member fetch denied; chunk flood limited; interleaved chat during a 25 MiB transfer still delivered promptly. (Spec written by architect; implementation delegable.) | **BULK** (with architect review of the test spec + result) | M |
| 3.4 | Attachment metadata in history: extend `send_room_history` payload with `attachments` filtered to surviving message ids; extend the frontend history ingest to consume it. | **TASTE** (protocol + trim invariant) | S |

Verify: `cargo test` in `src-tauri/`; manual two-instance smoke on LAN.

### Phase 4 — Tauri command boundary
| # | Task | Mark | Size |
|---|---|---|---|
| 4.1 | Commands: `send_message_with_attachments` (host + client variants, mirroring the existing send split), `fetch_attachment(attachment_id)` (client wire pull / host DB read), `get_attachment_bytes(sha256)` returning raw bytes via `tauri::ipc::Response`, `save_attachment(sha256, suggested_name)` (dialog + Rust-side write). TS invoke wrappers. | **TASTE** (Rust ↔ TS contract) | M |

Verify: `npm run typecheck && cargo check`.

### Phase 5 — Frontend UI
| # | Task | Mark | Size |
|---|---|---|---|
| 5.1 | `useAttachments` hook: transfer/progress state, object-URL lifecycle, auto-fetch policy (images ≤ 512 KiB), capability flag from Identity `features`. | **TASTE** | M |
| 5.2 | Composer: attach button, drag-drop, paste, chips, cap validation, disabled state on old hosts. | **TASTE** (user-facing) | M |
| 5.3 | Message rendering: inline image preview, file card, download action, progress/failure/retry states. | **TASTE** (user-facing) | L |
| 5.4 | Vitest coverage for the auto-fetch policy + size-format/sanitize display utils. | **BULK** | S |

Verify: `npm run typecheck && npm run lint && npm run test && npm run build`.

### Phase 6 — Lifecycle, docs, hardening
| # | Task | Mark | Size |
|---|---|---|---|
| 6.1 | Wire attachment GC into the `Delete` handler; startup + hourly orphan-blob sweep. | **TASTE** (sockets.rs handler glue; GC queries already BULK'd in 2.1) | S |
| 6.2 | ADR-0005 + gap-analysis updates (§8 entries) + CHANGELOG. | **TASTE** | S |
| 6.3 | Adversarial review pass on Phases 3–4 (Codex, explicit challenge: allocation-before-validation, gate bypasses, nonce/write-path ordering, old-peer coexistence), fable adjudicates. | per review policy | S |

Verify: full suite — `npm run typecheck && npm run lint && npm run test && npm run build`; `cargo test && cargo check` in `src-tauri/`.

---

## 9a. Latent traps recorded during Phase 1 review (must-do in later phases)

From the architect escalation review of the Phase 1 diff — each is invisible today (attachments are always `None`) and bites when the named phase lands:

1. **Validate inbound `attachments` on Chat frames (Phase 3).** The relay clones the whole `Message`, so unvalidated refs propagate by default. Before persist/relay: every ref must resolve to a complete uploaded blob, count ≤ `MAX_ATTACHMENTS_PER_MESSAGE`, `size` matching the blob row — else clients render unfetchable/spoofed cards.
2. **`save_message_internal` persists text fields only** — inserting the `attachments` sidecar rows in the Chat handler is a separate, easy-to-forget step (§2 step 5).
3. **The `Delete` broadcast clones the message and clears only `message`** — clear `attachments` there too (Phase 3/6), or the "deleted" broadcast carries the refs.
4. **History divergence window:** until task 3.4 lands, live messages show attachments but scrollback (built from DB rows) won't. Also re-check `send_room_history`'s trim budget once attachment metadata joins the payload.
5. **`blob_store` transaction ordering (Phase 2):** the query pool enforces `foreign_keys=ON`, so `store_blob` must insert the `attachment_blobs` parent row before chunk rows in its transaction.

From the Phase 2 architect review:

6. **The Delete-handler GC must be age-gated too (Phase 6).** `delete_orphan_blobs(pool, None)` (and the raw §5 `NOT IN` SQL) can sweep a fresh blob sitting in another user's upload→Chat gap; their Chat's sidecar insert then hard-fails on the FK. Scoping to the deleted message's shas alone is insufficient — dedup means an in-flight upload can share a sha with a just-deleted message. Use a short age gate (e.g. 1 hour) in the Delete handler as well.
7. **Verify the declared sha BEFORE calling `store_blob` (Phase 3).** `store_blob` computes and returns the true hash; persist-then-compare would store mismatched garbage under its real hash until the hourly sweep — a disk-fill primitive. The upload handler must hash the assembled bytes and compare against the declared sha first (the store's internal re-hash is cheap redundancy).
8. **Host-wide download concurrency cap (Phase 3).** §2 caps uploads at 4 host-wide but sets no host-wide download cap (`read_blob` materializes ≤25 MiB per fetch; N connections × 2 downloads each is unbounded memory). Add a host-wide cap in the 3.1 spec. Also: `read_blob` is transactional (snapshot), so a fetch racing GC returns `Ok(None)` — the fetch handler should map that to attachment-gone, and still treat a `Db("corrupt")` error as a server-side problem, not attachment-gone.

From the Phase 3 architect review:

9. **✅ RESOLVED — Sha grants retrieval via Chat-ref laundering.** Was: `validate_chat_attachments` checked existence+size only, so any member who learned a sha could reference it in their own room and fetch it, and the `UploadStart` dedup was an existence oracle. **Fixed** (commits closing this): a sender may reference sha S only if they hash-proved possession (persistent `attachment_blob_uploaders` table, migration v15, recorded on upload completion) OR S is visible to them in an accessible room (`sha_referenced_in_accessible_room`, sharing the `room_access_predicate!` macro with `room_join_allowed_internal` so the two can't drift). Enforced at both the reference gate (`validate_chat_attachments` with a typed `RefActor::{Host,Client}`) and the `UploadStart` dedup fast-path, both **oracle-equalized** on reply shape, timing (`may_reference_sha` runs unconditionally before the existence branch), and error string ("exists-but-not-yours" is byte-identical to "absent"). The host is the trusted authority and is exempt from the reference gate (unreachable from the wire). "Knowing a hash grants nothing" holds again. Reviewed by fable-5 (adversarial oracle-probe) + sonnet-5; the two residual timing signals fable found were then equalized. Deferred sub-item (accepted, negligible): a sha referenced in K rooms the requester *can't* see costs K indexed predicate evals vs 0 for an unreferenced sha — dominated by network RTT, requires the hash to have already leaked.
10. **FetchBegin is trusted unconditionally on the client (Phase 4).** Add a pending-fetch registry (only accept FetchBegin for attachment ids the client actually requested) plus a small assembly-count cap. Also fixes the duplicate-concurrent-fetch-of-one-sha self-destruct (two rapid fetches of one sha kill both assemblies) and bounds a malicious host to ~cap × 25 MiB of client memory.
11. **Sidecar insert races recipient auto-fetch (Phase 4/5).** The Chat handler distributes before the spawned save task inserts the `attachments` rows, so a recipient auto-fetch can get a spurious retryable "no longer available". Insert the sidecar rows synchronously before distributing attachment-bearing Chats.
12. **`reset_client` fails downloads silently (Phase 5 obligation).** Assemblies dropped on reconnect emit neither `attachment_failed` nor `attachment_ready` — the UI must fail in-flight fetch states on `connection_lost` and/or run a fetch timeout.
13. **Download slots can be pinned for minutes by a hard-disconnected peer (Phase 6).** `stream_blob_to_client` releases its slot only when a send fails; a black-holed TCP write can pin all 8 slots (+200 MiB). Add a per-send timeout or connection-generation check. Realized host memory budget: 100 MiB uploads + 200 MiB downloads ≈ **300 MiB worst case** (the 8-slot download figure was implementer-chosen; the design had fixed only the upload half).

From the Phase 4 review (reviewer-sonnet PASS + fixes in the review commit; the deeper architect/Codex escalation could not run — Anthropic session limit + Codex auth failure — so the adversarial pass was done by the main loop and is noted as such):

14. **`upload_attachment` reads a webview-supplied path (accepted, documented).** Safe only under the current webview-trust model (strict CSP, own bundled JS, no `dangerouslySetInnerHTML` → no XSS injection point). It extends a hypothetical webview compromise from chat data to arbitrary file read + exfiltration-to-host. If untrusted rendering is ever added, move the picker Rust-side (open the dialog in the command, as `save_attachment` already does); drag-drop paths would remain path-based. Doc comment on the command records this.
15. **Mid-stream download failures now surface (fixed in the review commit).** A bad/oversize/undecodable/out-of-order chunk previously tore down the client assembly silently; it now emits `attachment_failed` via `fail_download`, so the card fails retryably instead of spinning. Trap 12 (fail in-flight fetches on `connection_lost` + a fetch timeout) is still a Phase 5 UI obligation for the reconnect case.

Self-review confirmed clean (no code change needed): the persist-before-distribute reorder is idempotent on a retried `message_id` (messages `ON CONFLICT(message_id) DO NOTHING`, attachments `ON CONFLICT(id) DO NOTHING`) and cannot double-save (early return); `save_attachment` writes only the dialog-returned path (the sanitized `suggested_name` merely pre-fills the picker, can't influence the directory); the pending-fetch cap-slot lifecycle has no permanent leak; mode-flip mid-transfer yields at worst a benign stale "not available" error (no panic/deadlock), and reconnect clears client transfer state.

From the Phase 5/6 review (SHOULD-FIX items 1–3 and NICE-TO-HAVE 6/8-partial applied in the review commit; these remain as recorded follow-ups):

16. **Dedup fast-path doesn't refresh `created_at` (NICE-TO-HAVE).** The age-gated sweep gates on creation time, but a blob re-referenced via the `UploadStart` dedup fast-path keeps its old timestamp. Negligible-probability failure: an old blob, momentarily orphaned by a just-deleted message and re-shared within the hour, could be swept in the sub-second window between a new Chat's validate and its sidecar insert (that one message's sidecar insert then FK-fails; no corruption). Fix if convenient: touch `created_at` when a blob gains a new `attachments` reference.
17. **Slow-drip download peer (NICE-TO-HAVE).** `send_frame_timed`'s 30s budget is per-frame; a peer that keeps each ~60 KB frame just under 30s can hold a download slot + up to 25 MiB for a long time (slowloris-style, bounded to 8 slots / ~200 MiB, authenticated room members only). Trap 13 covered the fully black-holed peer; add a whole-transfer deadline or min-throughput check to close the drip variant.
18. **Image layout stability / dimensions (NICE-TO-HAVE, §7).** `AttachmentRef.width/height` are wired end-to-end but never populated — the composer would need an upload-time image-dimension probe (both dialog and drag-drop paths). Until then previews reserve no aspect-ratio space (minor layout shift on load); the preview now uses `object-contain` so at least nothing is cropped.

Backlog (pre-existing, surfaced during review): the client listener logs the full plaintext of every non-attachment frame at info level (`sockets.rs` "Client received") — against the never-log-plaintext spirit; fix as its own change.

Conscious deviation recorded: §9 task 2.1 said `read_blob` "streamed by seq"; the implementation materializes the full blob (bounded by the 25 MiB cap — the same budget §2 accepts for upload buffering). A chunked/streaming read variant can be added beside it later without breaking callers.

Backlog note (pre-existing, not attachment-specific): migration batches run in autocommit and the `_migrations` row is inserted after the batch, so a crash mid-batch leaves partial DDL that fails on retry (`CREATE TABLE` without `IF NOT EXISTS`). Applies to v9/v11/v14 alike; fix as a convention change, not in this feature.

---

## 10. Explicit judgment calls

1. **The "10MB frame chunking" premise inverted:** frames aren't chunked at the transport — the Noise 64 KB record is the true cap and nothing today can send a frame bigger than that (§0). The design chunks at 44 KiB accordingly.
2. **Rejected the separate blob channel** — it duplicates session/auth surface for a problem (HOL blocking) that 64 KB chunking already bounds.
3. **Rejected filesystem blob storage** despite it being the conventional choice — the SQLCipher-inherited at-rest encryption and the elimination of the path-traversal class outweigh DB-bloat concerns at a 25 MiB cap; the storage layer is swappable later without touching the wire protocol.
4. **"Eager push" reframed as client auto-fetch** so the host has exactly one, membership-gated egress path for blob bytes.
5. **No `PROTOCOL_VERSION = 2`** — the coexistence risk lives in unknown `MessageType` variants (old host drops the connection on parse failure), which a version bump wouldn't fix but capability negotiation does.
