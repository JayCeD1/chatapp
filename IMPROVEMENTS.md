# Nutler — Improvement Roadmap

> Authoritative, de-duplicated reference for turning Nutler from a working LAN-chat prototype into a professional, shippable team-chat product. Findings consolidated from a 7-dimension specialist audit, with `file:line` evidence verified against the source. Severity reflects user/security impact; effort is rough (S/M/L).
>
> _Living document — check items off as they land, and link PRs next to each task._

---

## State of the project

Nutler is a **functional, well-structured prototype** with a real architecture underneath it — not a toy. Credit where due:

- **Dual host/client model.** A single app can act as the LAN host (`server_listen_as_participant`) or a client (`client_connect_to_server`), with the host modeled as a participant. This is the right shape for a peer-hosted LAN tool.
- **Length-prefixed framing protocol.** Messages are framed with a 4-byte big-endian length prefix over `tokio` TCP, and the **server read path is hardened** with a 10MB cap and zero-length guard (`sockets.rs:280,285`). The framing concept is sound.
- **Typed, indexable identity.** `server_streams` is keyed by `user_id` for O(1) lookups, and every wire `Message` already carries a stable `message_id: String` (UUID) and structured fields (`sockets.rs:34,55`).
- **Real migrations.** A 7-step migration set with seeded departments/rooms and FK declarations (`migration.rs`), run through `tauri-plugin-sql`.
- **Polished glassmorphism UI.** A cohesive dark aesthetic with animated entrances and an emoji picker.

**The honest gaps.** The app is one global mutable `AppState` that conflates the host's own identity with the multi-client server; teardown is effectively absent (the disconnect commands exist but the frontend never calls them); the DB is wired through **two pools pointing at potentially different files**; timestamps are **inconsistent between live and persisted messages**; there is **no security model at all** (plaintext, unauthenticated, identity is whatever the client asserts); and there are **zero automated tests**. Many "features" are half-wired — the infrastructure exists (`discover_servers`, `update_user_online_status`, `UserInfo`, `RoomLeave`, `message_id`, `is_private`/`created_by`) but is never invoked end-to-end, so the product reads as a demo rather than a tool.

The good news: a large fraction of the work is **completing wiring that's already there**, not greenfield.

---

## 1. Critical bugs / correctness

These are *actually broken* today and visible to users or destabilizing under normal use.

### 1.1 Logout never tears down TCP/sockets; disconnect commands are dead code
**What's wrong:** `logout()` only calls `leaveRoom()` (a DB-only `UPDATE`) and clears React state — it never invokes `client_disconnect` or `server_participant_disconnect`, both of which are implemented and registered (`lib.rs:95-96`). Confirmed: the source comment at `useChatConnection.ts:254` literally says "Disconnects aren't strictly exposed… relying on window close or reload."
**Evidence:** `useChatConnection.ts:249-261`; commands at `sockets.rs:922`/`972`, registered `lib.rs:95-96`.
**Impact:** Every logout leaks the client write half + reader task; the host keeps `0.0.0.0:3625` bound and the accept loop alive for the process lifetime. Re-login → "address already in use." Server-side `room_clients`/`server_streams` entries leak (ghost users inflating counts).
**Fix:** In `logout()`/`leaveRoom()`, before clearing state: `if (mode === 'server') await invoke('server_participant_disconnect'); else await invoke('client_disconnect');` (try/catch). Store the host listener's shutdown via a `oneshot`/`CancellationToken` in `AppState` so the port frees. Cancel the client listener task on disconnect.

### 1.2 Disconnect commands emit a null payload onto the chat channel
**What's wrong:** Both disconnect commands end with `let _ = app.emit("message", ())` (`sockets.rs:966,1032`). The unit `()` serializes to JSON `null`. The frontend `message` listener does `JSON.parse(e.payload)` and then dereferences `m.room`, `m.username`, etc. (`useChatConnection.ts:70-83`).
**Evidence:** `sockets.rs:966`, `sockets.rs:1032`; listener `useChatConnection.ts:68-99`.
**Impact:** The moment these commands are wired up (see 1.1), every logout pushes a `null` onto the channel the UI treats as chat. It's swallowed in `catch` but pollutes the contract and the intended "notify UI of disconnect" never happens.
**Fix:** Stop overloading `"message"` for lifecycle. Emit a dedicated event (`"disconnected"` / `"server_stopped"`) with a structured payload, or a properly serialized Disconnect `Message` string. Never emit `()` on a JSON-parsed channel.

### 1.3 Two-pool DB race: migrations and queries can hit different files
**What's wrong:** `tauri-plugin-sql` runs migrations against the **relative** `"sqlite:nutler.db"` (`lib.rs:39`), while a **separate** `SqlitePool` is created in a detached `spawn` against an **absolute** `app_data_dir` path *after* `setup` returns (`lib.rs:52-63`). Two issues: (a) different URLs → migrations land in one file, queries run against another → "no such table: users"; (b) the pool is `manage`d inside an un-awaited spawn, so an early command hits "state not managed."
**Evidence:** `lib.rs:37-41` (plugin), `lib.rs:52-63` (spawned sqlx pool), commands take `State<'_, SqlitePool>`.
**Why it "works on your Mac":** On macOS `app_config_dir` and `app_data_dir` resolve to the same `~/Library/Application Support/<id>` path, so the two files coincide. On **Linux/Windows they diverge** (config vs data/roaming) → the queried DB has no tables. Classic flaky-on-some-machines bug.
**Fix:** Pick ONE database owner. Build the sqlx pool **synchronously in `setup`** with `SqliteConnectOptions::new().filename(db_path).create_if_missing(true)` (block_on before returning so it's managed before any command runs), point the plugin at the **same absolute path**, and run migrations on the one pool. (Or drop sqlx and route everything through the plugin.)

### 1.4 Timestamp representation differs between live and persisted messages
**What's wrong:** DB `created_at` is `DEFAULT CURRENT_TIMESTAMP` → a UTC **string** like `'2026-06-27 12:34:56'` (`migration.rs:76`), returned raw by `get_room_messages`. The socket `Message.created_at` is a **u64 epoch-seconds** (`sockets.rs:58`). The frontend converts only socket messages via `new Date(Number(m.created_at)*1000).toISOString()` (`useChatConnection.ts:82`) and passes DB history through untouched (`setMessages(msgs)`, `useChatConnection.ts:55`). `ChatInterface.formatTime` then does `new Date(isoString)` — and `new Date('2026-06-27 12:34:56')` is parsed as **local** time though the value is UTC.
**Evidence:** `migration.rs:76`, `sockets.rs:58`, `useChatConnection.ts:55,82`, `ChatInterface.tsx:51-53`.
**Impact:** Every loaded room renders history shifted by the local UTC offset, sometimes `Invalid Date`. The dedup heuristic (1.5) compares across the two formats → `NaN` comparisons → silent failures.
**Fix:** Standardize on **one** wire format end-to-end. Recommended: store ISO-8601 UTC (`DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))`) and make the socket `Message.created_at` a `String` too; then both paths are identical and the frontend needs no per-source branching. Run **both** sources through one shared `normalizeMessage()`.

### 1.5 Message dedup is a lossy heuristic; the stable `message_id` is thrown away
**What's wrong:** The backend generates a UUID per message (`sockets.rs:55`), but `types.ts` `Message` has no `message_id`, the normalizer never copies it, and dedup matches on `message===message && username===username && |Δt|<1000ms` (`useChatConnection.ts:88-93`).
**Evidence:** `sockets.rs:55`, `types.ts:22-31`, `useChatConnection.ts:74-93`.
**Impact:** Two legitimate identical messages within 1s (user types "ok" twice) collapse into one — **real data loss**. Conversely it can't reliably dedup the echo-vs-rebroadcast it was written for once timestamps drift. No idempotency key means retried saves create duplicate rows.
**Fix:** Add `message_id TEXT` to the `messages` table with a `UNIQUE` index; thread it through `save_message_internal` (`INSERT … ON CONFLICT(message_id) DO NOTHING`); return it from `get_room_messages`; add it to `types.ts`; dedup by id; use it as the React key (replaces `key={idx}`, see 5.7).

### 1.6 Message ordering is non-deterministic at second resolution
**What's wrong:** `get_room_messages` does `ORDER BY m.created_at DESC LIMIT 50` then `.reverse()` in Rust. `created_at` has 1-second resolution, and the socket layer saves in parallel spawns, so same-second messages have undefined order — and `DESC+LIMIT+reverse` can truncate the *wrong* messages on a tie.
**Evidence:** `db_queries.rs:421-445`, `migration.rs:76`.
**Fix:** `ORDER BY m.created_at DESC, m.id DESC` for a total tie-break; combine with sub-second timestamps (1.4). Consider selecting newest-50 in a subquery ordered ASC to drop the in-Rust reverse.

### 1.7 Reconnect re-runs the full Connect handshake → duplicate/ghost sessions
**What's wrong:** On `connection_lost` the frontend calls `client_connect_to_server` again, which opens a new `TcpStream`, sends a fresh `Connect`, overwrites `client_stream`, and spawns **another** listener — while the host treats every `Connect` as a brand-new registration with no dedup (`insert` + `push` into the room Vec).
**Evidence:** `useChatConnection.ts:128-135`; host registration `sockets.rs:310-318`; client connect spawns listener.
**Impact:** Duplicate room-Vec entries (every broadcast delivered twice), repeated "🔵 X joined" ghost-join spam, leaked reader tasks, and a teardown/reconnect race where the old half-open EOF runs `clean_client` and removes the user who just reconnected.
**Fix:** Make `Connect` idempotent on the host (if `user_id` already in `server_streams`, replace the writer + dedup the room Vec, suppress the join broadcast). On the client, hold a `JoinHandle`/`AbortHandle` in `AppState` and cancel/await the previous listener before spawning a new one. Add `MessageType::Reconnect` (or an "already known" flag) so the host can distinguish.

### 1.8 `RoomLeave` is defined but never constructed; leaves are invisible to peers
**What's wrong:** `MessageType::RoomLeave` exists (`sockets.rs:68`) but is never built anywhere; `leaveRoom()` only does the DB `UPDATE`. The server `handle_server_message` match has `_ => {}` so client `Disconnect`/`RoomLeave` are silently dropped.
**Evidence:** `sockets.rs:68` (defined), only referenced in a comment at `sockets.rs:890`; `useChatConnection.ts:223-233`.
**Impact:** When a user leaves, the host's `room_clients` still lists them, so the host keeps relaying that room to them and peers never see a "left." Frontend state and backend routing diverge.
**Fix:** Implement `client_leave_room`/`server_leave_room` that construct a `RoomLeave` `Message`, update `room_clients`, and broadcast; invoke from `leaveRoom()` alongside the DB write. Handle `Disconnect`/`RoomLeave` explicitly in the server match. Mirror the existing `client_join_room` flow.

### 1.9 Stale captured `current_room` on cleanup leaves dangling room members
**What's wrong:** `RoomJoin` updates the stored `ClientConnection` in `server_streams`, but `clean_client` removes the user using the `client` value **captured at Connect time**, which is never updated after a `RoomJoin`.
**Evidence:** `sockets.rs:298-306,339-363` (capture + cleanup) vs the updated entry on `RoomJoin`.
**Impact:** Join A → move to B → disconnect removes the user from A (stale), leaving a permanent dangling `user_id` in B's Vec; `user_count`/presence permanently wrong.
**Fix:** On disconnect, re-read the current entry from `server_streams` (which `RoomJoin` keeps current) to find the real room — don't trust a snapshot.

### 1.10 `joinRoom` passes `oldRoom = department_name`, not the actual previous room
**What's wrong:** `joinRoom` passes `oldRoom: currentUser.department_name` (with an in-code "Simplification" comment) although `currentRoom` holds the real previous room.
**Evidence:** `useChatConnection.ts:204`; backend uses `old_room` to remove from `room_clients`.
**Impact:** Moving A→B never removes the user from A's `room_clients` → ghost membership, A's broadcasts keep arriving.
**Fix:** Pass `oldRoom: currentRoom?.name`, falling back to `department_name` only on first join.

### Critical/correctness matrix

| Item | Type | Severity | Effort |
|---|---|---|---|
| 1.1 Logout never tears down sockets | bug | critical | M |
| 1.2 Disconnect emits null payload | bug | high | S |
| 1.3 Two-pool DB race / wrong file | bug | critical | M |
| 1.4 Timestamp inconsistency (live vs DB) | bug | critical | M |
| 1.5 Dedup ignores `message_id` (data loss) | design | high | M |
| 1.6 No id tie-break in ordering | bug | high | S |
| 1.7 Reconnect duplicates sessions | bug | high | M |
| 1.8 `RoomLeave` never sent | bug | medium | M |
| 1.9 Stale cleanup leaves dangling members | bug | medium | M |
| 1.10 `oldRoom` = department, not prev room | bug | low | S |

---

## 2. Security & Auth

> Nutler currently has **no security model**: the host binds `0.0.0.0:3625` and accepts plaintext, unauthenticated, length-prefixed JSON from any LAN peer, trusting client-asserted `user_id`/`username`/`room`. These are foundational design gaps, not isolated bugs.

### 2.1 No authentication — any LAN peer can assert any identity *(critical)*
The host registers a client purely from the `Connect` message: `username/user_id/current_room` taken verbatim (`sockets.rs:297-319`), bind at `sockets.rs:163`. No shared secret, room password, token, or allowlist. Anyone on the LAN can send one frame claiming `user_id:1, username:"CEO"` and be that user.
**Fix:** Require a host-set shared secret/room password in a handshake before the first message; reject if missing/wrong. Longer term, issue per-user signed tokens (HMAC/asymmetric) at login and verify on every frame. Keep a server-side connection→identity map and **ignore client-asserted identity fields after handshake**.

### 2.2 No message integrity/authorization — spoofed sender persisted *(critical)*
`handle_server_message` takes `message.user_id`/`room_id` from the wire and persists via `save_message_internal(msg_clone.user_id as i64, …)` with no check the connection owns that id; rebroadcast verbatim.
**Evidence:** `sockets.rs:501-522`, DB write `db_queries.rs:388-399`.
**Fix:** Derive `user_id`/`username` server-side from the authenticated binding; drop frames whose claimed id/room don't match. Persist server-resolved identity only.

### 2.3 Plaintext transport leaks messages + PII *(high)*
`send_message_with_length` writes raw `serde_json::to_string` bytes over a bare `TcpStream` (`sockets.rs:905-919`); no TLS/Noise dependency in `Cargo.toml`. Emails are stored in plaintext (`migration.rs:24`) and identity rides every frame.
**Fix:** Wrap streams in `tokio-rustls` (host self-signed cert, pinned) **or** a Noise (`snow`) handshake keyed by the room password — solving auth + encryption together.

### 2.4 Client read loop allocates an attacker-controlled buffer with no cap *(high — also a hard crash)*
`start_client_listener` does `let msg_len = u32::from_be_bytes(len_bytes) as usize; let mut message_buffer = vec![0u8; msg_len]` with **no cap and no empty-frame guard** (`sockets.rs:749-750`) — unlike the server path which caps at 10MB (`sockets.rs:285`). A malicious host sends `0xFFFFFFFF` → ~4GB allocation → OOM/hang on every client.
**Fix:** Mirror the server guards (reject `==0`, reject `>MAX_FRAME`). **Factor a single `read_frame()` used by both paths** so the cap can never drift. Define one shared `MAX_FRAME` constant.

### 2.5 Unbounded accept loop, no rate/connection limits *(high — DoS)*
`loop { accept(); spawn(handle_client_connection) }` with no concurrency cap, per-IP limit, rate limit, or read timeout (`sockets.rs:230-253`). Combined with no auth, a peer can exhaust FDs/memory and bloat `room_clients`/`server_streams`.
**Fix:** `Semaphore`-bounded accept loop, per-IP cap, token-bucket inbound rate limit, per-frame `tokio::time::timeout`, cap distinct rooms/users per connection.

### 2.6 Remote-triggerable panics on untrusted input *(high)*
`serde_json::to_string(...).unwrap()` and `SystemTime::duration_since(UNIX_EPOCH).unwrap()` in broadcast paths (`sockets.rs:462,594-597`); `acquire_owned().await.unwrap()` (`sockets.rs:119`); `lib.rs:58` `.expect` on DB connect inside a spawn; `lib.rs:100` `.expect`. A panic in `distribute_message_to_all` aborts the broadcast mid-loop.
**Fix:** Replace with `?`/`match`/`unwrap_or_default()` in all network/broadcast paths; validate deserialized `Message` fields before acting. Add a `now_secs()` helper.

### 2.7 Webview granted `sql:allow-execute` with CSP disabled *(high)*
`capabilities/default.json` grants `sql:default` **and** `sql:allow-execute`; `tauri.conf.json` sets `"csp": null`. Any injected script (XSS via unescaped peer message text — far more likely with null CSP) can run arbitrary SQL against `nutler.db`.
**Evidence (verified):** `capabilities/default.json` permissions list, `tauri.conf.json:26`.
**Fix:** Remove `sql:allow-execute` (route all DB access through the vetted `#[tauri::command]` functions). Set a strict CSP, e.g. `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost`. Ensure chat content renders as text (never `dangerouslySetInnerHTML`). Remove `dialog:default` if unused.

### 2.8 No input validation/length limits at command boundary *(high)*
`upsert_user`/`create_user`/`save_message` accept arbitrary unbounded strings (`db_queries.rs:54-133,352-405`); only the 10MB frame cap exists.
**Fix:** Enforce caps (username ≤ 64, email ≤ 254 + format, message ≤ a few KB); reject (don't silently truncate) oversized/invalid input; add `NOT NULL`/`CHECK` constraints.

### 2.9 Room routing by client-asserted name, no membership check *(medium)*
`room_clients` keyed by free-text `message.room` (`sockets.rs:38,314-318`), used as the broadcast target; `RoomJoin` never verifies authorization; `is_private` is never enforced.
**Fix:** Route by server-validated numeric `room_id`; enforce `is_private`/`user_rooms` membership server-side before adding to `room_clients`; add `UNIQUE` on `chat_rooms.name`.

### 2.10 `discover_servers` is an unconsented LAN port scan with no handshake *(medium)*
Connects to `:3625` across ~600 hardcoded IPs, labels **any** open port a "Chat Server" with fabricated `user_count:0`/name (`sockets.rs:96-146`). Can trip IDS/IPS and false-positive any service; a fake `:3625` can lure clients into the no-cap read DoS (2.4).
**Fix:** Replace active scanning with mDNS/DNS-SD or UDP broadcast announce/respond; if kept, gate behind explicit consent, derive the local subnet, and require a signed/versioned handshake before listing.

### 2.11 Emails (PII) stored + transmitted in plaintext *(medium)*
`email TEXT UNIQUE` plaintext (`migration.rs:24`); unencrypted `nutler.db` + cleartext TCP.
**Fix:** Encrypt at rest (SQLCipher / OS-keychain key), restrict file perms, minimize PII in network payloads, combine with TLS (2.3).

---

## 3. Backend / Networking

### 3.1 Single global identity state corrupts the host's own room/user *(high — root cause)*
`AppState` stores **one** `username/user_id/current_room/current_room_id` (`sockets.rs:41-46`). `send_as_server_participant` and `server_participant_join_room` derive identity/room from these globals, but `server_streams` holds N clients and `room_clients` is keyed by room **name**. The host is modeled as both "the server" and "a participant" using scalars designed for the single-connection client role.
**Fix:** Split into (a) immutable per-connection identity in `ClientConnection` (already present) and (b) a host-local "self participant" record. Server broadcast must use `ClientConnection` fields + an explicit `host_user_id`, never the mutable scalars. **Key `room_clients` by `room_id: u64`, not name.** This is the architectural root behind 1.7, 1.9, 2.9, and the `user_id`-collision skip bug.

### 3.2 Lock-ordering inconsistency / locks held across emit and I/O *(high)*
`server_streams` then `room_clients` are taken as nested `tokio::sync::Mutex` guards in several places (`sockets.rs:310-318,417-418,526-527`). No opposite-order path exists today, but `distribute_message_to_all` holds **both** for the entire fan-out loop **including `app.emit` (462) and spawned sends** — serializing all delivery/registration/joins behind one global critical section, and any future `room_clients`-before-`server_streams` introduces a classic AB/BA deadlock.
**Fix:** Document one lock order (always `server_streams` before `room_clients`). In `distribute_message_to_all`, collect target ids + writer `Arc`s while briefly holding the locks, **drop both guards**, then emit and spawn. Never hold either Mutex across `.await` network I/O or `app.emit`.

### 3.3 Broadcast sends are fire-and-forget; dead clients never evicted; reordering *(medium)*
Each recipient gets a detached `spawn` that on failure only `println!` (`sockets.rs:451-457`) — no removal from `server_streams`/`room_clients`. Half-open connections linger as ghosts; per-recipient detached spawns also **reorder** back-to-back messages racing for the writer lock.
**Fix:** Send sequentially per recipient (or via a per-connection mpsc writer task preserving order). On write error, mark for removal and run the EOF cleanup path. Add a write timeout + periodic heartbeat/ping to detect half-open connections.

### 3.4 `tokio` Cargo features omit `net`/`time`/`sync`/`io-util` it directly uses *(low)*
`Cargo.toml:28` declares only `rt`, `rt-multi-thread`, `macros`; compiles only because `tauri`/`sqlx` transitively enable `tokio/full`.
**Fix:** Declare `net`, `time`, `sync`, `io-util` (or `full`) explicitly so the build is self-describing.

### 3.5 Host self-join can collide with a client's `user_id` *(low)*
Host pushes its own `user_id` into `room_clients` (`sockets.rs:181-182`); broadcast skips host via `Some(user_id) == server_user_id` (`sockets.rs:442`). If a client's id ever equals the host's, that client's messages are silently dropped.
**Fix:** Track the host participant separately; key `room_clients` by `room_id` (covered by 3.1).

---

## 4. Data Model (SQLite)

### 4.1 No `message_id` column *(high)* — see 1.5
Add `message_id TEXT` + `UNIQUE` index; thread through save/load/types; enables reliable dedup, idempotent inserts, edit/delete targeting, and stable React keys.

### 4.2 No indexes on hot lookups *(high)*
Zero `CREATE INDEX` anywhere (`migration.rs`). `get_room_messages` filters/sorts on `(room_id, created_at)`; room-list subqueries scan `user_rooms WHERE is_active=1`.
**Fix:** `CREATE INDEX idx_messages_room_created ON messages(room_id, created_at, id); CREATE INDEX idx_messages_user ON messages(user_id); CREATE INDEX idx_user_rooms_room_active ON user_rooms(room_id, is_active);` The first serves `ORDER BY … DESC LIMIT 50` directly.

### 4.3 No `UNIQUE` on `chat_rooms.name` though broadcasting keys by name *(high)*
`name TEXT NOT NULL` with no UNIQUE (`migration.rs:36`); `room_clients` keyed by name. Two same-named rooms collapse into one broadcast bucket → cross-room leakage; the down-migration `DELETE … WHERE name LIKE '% General'` (`migration.rs:118-120`) would nuke unrelated user rooms.
**Fix:** Add `UNIQUE` (or unique index) on `chat_rooms.name`; better, key routing by `room_id` (3.1).

### 4.4 FKs declared but not enforced; no `ON DELETE` behavior *(medium)*
No `PRAGMA foreign_keys=ON` on either connection; plain `SqlitePool::connect` leaves FK enforcement off. No `ON DELETE`/`ON UPDATE` rules → orphan rows, and `get_room_messages`' `INNER JOIN users` silently drops messages whose author row is missing.
**Fix:** `foreign_keys(true)` in `SqliteConnectOptions` + plugin pragma; choose semantics (messages/user_rooms `ON DELETE CASCADE`; `created_by`/`department_id` `ON DELETE SET NULL`). Also set `journal_mode=WAL` + `busy_timeout=5000` so the two writers don't hit "database is locked."

### 4.5 `is_emoji`/`message_type` fidelity lost at persistence *(medium)*
Every `save_message_internal` from the socket layer passes literal `false` for `is_emoji` (e.g. `sockets.rs:511,612`), discarding the real flag; `message_type` is hand-typed strings.
**Fix:** Pass `msg_clone.is_emoji` and derive `message_type` from the enum.

### 4.6 `upsert_user` trusts client email, no normalization *(medium)*
Keyed on `email` (NULLable, not lowercased/trimmed). Empty string `''` is a valid value → every blank-email login merges onto one shared row; `A@x.com`/`a@x.com` fragment into two users. `user_id` is the broadcast/attribution key, so this corrupts authorship.
**Fix:** Reject/synthesize empty email; trim+lowercase; `email NOT NULL UNIQUE`; use atomic `INSERT … ON CONFLICT(email) DO UPDATE … RETURNING *` instead of read-then-write-then-read.

### 4.7 Control events persisted as chat rows *(low)*
Connect/Disconnect/RoomJoin all written to `messages` (`sockets.rs:203-216,484-497,552-565,386-394`); `get_room_messages` has no `message_type` filter, so the 50-row LIMIT fills with system noise reloaded forever.
**Fix:** Stop persisting transient control events, or `WHERE message_type='Chat'` on load; longer term add soft-delete (`deleted_at`) + read-state.

### 4.8 `join_room` uses `INSERT OR REPLACE`, churning PK + `joined_at` *(low)*
Re-joining deletes/re-inserts, changing `id` and resetting `joined_at` (`db_queries.rs:323-325`).
**Fix:** `INSERT … ON CONFLICT(user_id, room_id) DO UPDATE SET is_active=1`.

### 4.9 Duplicate migration version numbers for Up/Down *(low)*
Ups versioned 1-7 and Downs reuse 7..1 in one Vec (`migration.rs:114-168`), relying on the plugin filtering by kind.
**Fix:** Keep only Ups in the runtime list (plugin applies forward-only); manage Downs separately or with distinct versions.

---

## 5. Frontend Architecture (React 19 / TS)

### 5.1 Message listener gated on `currentRoom` drops messages for other rooms *(high)*
`if (!currentRoom) return;` and `if (m.room === currentRoom.name)` with effect deps `[currentRoom]` (`useChatConnection.ts:62-63,85,109`). Messages for any non-active room are discarded; the listener churns (unlisten/relisten) on every room switch, with a gap where messages are missed; in the rooms view (`currentRoom` null) **nothing** is received.
**Fix:** Register the `message` listener **once** (empty deps), keep a per-room store (`Map<roomId, Message[]>`), read `currentRoom` from a ref in the handler. Prerequisite for unread badges and notifications.

### 5.2 Reconnect effect: non-resetting retryCount, no cancellation, stale closures *(high)*
`retryDelay/retryCount/maxRetries` are local `let` inside the `connection_lost` callback; `setTimeout` id is never captured (never cleared on unmount); effect deps include `currentRoom` so timers fire with stale room captured at `useChatConnection.ts:133-134`.
**Fix:** Lift retry state into `useRef`; clear the timer in cleanup; drop `currentRoom` from deps and read latest from refs; reset `retryCountRef` only on confirmed reconnect. Consider a `useReducer` connection state machine (`connected|reconnecting|disconnected`).

### 5.3 All errors are `console.error` — zero user feedback *(high)*
Every catch logs and swallows (`useChatConnection.ts:34,43,57,99,189,219,230,244`; `LoginView.tsx:33`). `sendMessage` failure leaves the input cleared with the message gone. There is no `error` in the hook's API.
**Fix:** Add an `error`/`toast` state (or a small notification context); surface failures via toast/banner; don't clear the composer until `invoke` resolves.

### 5.4 `Message` type matches neither backend struct; bridged with `as any` *(medium)*
`types.ts:22-31` has `created_at: string`, no `id`/`message_id`; socket struct has `message_id` + u64 `created_at`; bridged via `as any` + `|| 0`/`|| false` fallbacks (`useChatConnection.ts:70-82`). The TS type is decorative; a backend rename won't fail the build.
**Fix:** Define one wire `Message` mirroring the socket struct and a separate normalized `UiMessage`; generate types from Rust (`ts-rs`/`specta`); remove `as any`.

### 5.5 Single 280-line god-hook *(medium)*
`useChatConnection` owns view routing, connection mode, auth, room/message state, three effects, and seven actions; components are pure props sinks. This shared closure scope is *why* the stale-closure/listener-churn bugs exist, and there's no seam to unit-test connection logic.
**Fix:** Split into `useConnection` (lifecycle + reconnect), `useMessages` (per-room store + listener), `useAuth/useSession`, and a small router. Consider a reducer or Zustand for the message map.

### 5.6 No optimistic / failed-send state *(medium)*
`sendMessage` awaits and only `console.error`s; the optimistic echo happens in **Rust** (`app.emit` of the sent message), so the UI message only appears via round-trip; on reject nothing renders and the text is already cleared.
**Fix:** Optimistic insert keyed by client-generated id with `status: pending|sent|failed`, reconciled against backend `message_id`; retry button; keep input until success.

### 5.7 `key={idx}` defeats reconciliation *(medium)* — fixed once `message_id` lands (1.5)
`ChatInterface.tsx:99`. Bulk-replace by `loadRoomMessages` + dedup skips shift indices → animations replay on wrong rows.

### 5.8 Presence is hardcoded *(medium)*
Static "Online" pill (`ChatInterface.tsx:78-80`); `user_count` is a one-shot DB snapshot never refreshed and unrelated to live sockets (`room_clients` not surfaced). The pill stays green even after `connection_lost`.
**Fix:** Expose `get_room_participants`/presence event from Rust (it has `room_clients`); drive the pill from real connection state; re-run `loadChatRooms` on RoomList focus.

### 5.9 Half-wired backend capabilities never invoked *(medium)*
`discover_servers` (registered `lib.rs:87`), `update_user_online_status` (`lib.rs:74`), and `get_user_by_id` (for session restore) are never called from `src/`. `localStorage('nutler.userId')` is write-only (`useChatConnection.ts:167`) — never read back to rehydrate.
**Fix:** Wire discovery into LoginView; call `update_user_online_status(true/false)` on login/logout; rehydrate session on mount via `get_user_by_id`.

### 5.10 No loading/empty states *(low)*
No loading flags during `loadChatRooms`/`loadRoomMessages`; no empty-case in `RoomList`/`ChatInterface`.
**Fix:** Per-fetch `isLoading` flags + skeletons; empty-state copy.

---

## 6. UI/UX & Accessibility

### 6.1 Three full-screen view swaps instead of a persistent layout *(high — biggest gap)*
`Chat.tsx:38-66` renders Login/RoomList/ChatInterface as mutually exclusive full-screen views; "Leave Room" pops navigation and destroys chat context. You can never see your rooms while chatting; there's no sidebar, no members panel.
**Fix:** One persistent shell after login — left sidebar (~280px, channels grouped by department, active highlight, unread badges) + center column (header + scroll + composer) + optional right members panel. `grid-cols-[280px_1fr_auto]`; switching rooms just swaps the center pane. Demote "leave room" to a context-menu action.

### 6.2 Accessibility is effectively nonexistent *(high)*
Confirmed zero `aria-*`, `<label>`, `role`, `sr-only` in app source. Icon buttons rely on `title` only (back/logout/emoji/send); all inputs are placeholder-only; the department `<select>` has no programmatic label; the emoji-picker buttons have no label.
**Fix:** `aria-label` on every icon button; visible or `sr-only` `<label htmlFor>` for every input/select; `role="log"` + `aria-live="polite"` on the messages container.

### 6.3 Errors swallowed to console — no toasts/inline errors/status *(high)*
Same root as 5.3 from the UX angle: a wrong IP or unreachable host just makes the form go quiet; `connection_lost` retries silently with no banner.
**Fix:** Toast/notification system; inline login/connection errors ("Could not reach 192.168.x.x:3625"); a persistent reconnection banner with manual Retry after max retries.

### 6.4 Animations ignore `prefers-reduced-motion` *(medium)*
Two infinite `animate-pulse` blurred blobs (`Chat.tsx:35-36`) + per-message slide-up; zero `prefers-reduced-motion` anywhere. WCAG 2.3.3 + continuous GPU cost.
**Fix:** Global `@media (prefers-reduced-motion: reduce)` reset; gate the blobs behind it in JS.

### 6.5 Low-opacity white text fails WCAG contrast *(medium)*
`text-white/30` timestamps, `text-white/50` metadata, faint placeholders over a semi-transparent glass gradient — under 4.5:1.
**Fix:** Raise body/metadata to white/70-80 on a solid panel behind text; bump 10px timestamps to 11-12px ≥ white/60; verify each pairing.

### 6.6 Message list lacks date separators, sender grouping, avatars, actions *(medium)*
Per-message bubbles with `key={idx}`, repeated sender names, no avatars/dividers; `group` class set up but no hover actions.
**Fix:** Date dividers; consecutive-sender grouping; initials/color avatars; stable `message_id` key; `group-hover` action toolbar (copy/react/reply); render emoji-only messages larger.

### 6.7 No empty states / skeletons / scroll-to-bottom *(medium)*
Blank grid on empty rooms; pop-in on slow fetch; `ChatInterface` force-scrolls on every message, yanking users away from history.
**Fix:** Empty-state copy + shimmer skeletons; "Jump to latest" button; only auto-scroll when already near bottom.

### 6.8 Fixed 800×600 window, no min size *(medium)*
`tauri.conf.json` width 800 / height 600, no `minWidth`/`minHeight`/`resizable` (verified). `h-screen` layouts can push the composer off-screen.
**Fix:** Default ~1100×720, `minWidth/minHeight` ~720×480; `h-dvh` + `min-h-0` on the scroll region; responsive sidebar collapse.

### 6.9 Generic placeholder chrome / discovery vaporware *(medium)*
`index.html` still titled "Tauri + React + Typescript" with `/vite.svg` favicon; login "logo" is a generic lucide `Globe`; README advertises "Discover Servers" but the UI requires hand-typing an IP.
**Fix:** Title "Nutler" + real favicon/app icon + wordmark; wire `discover_servers` into login as the primary onboarding path.

### 6.10 No settings/theme toggle (dark-only) *(low)*; login form has no Enter-to-submit / focus management *(low)*
Zero `dark:` variants; only post-login actions are leave/logout. LoginView isn't a `<form>`, so Enter does nothing; hidden IP field stays focusable; focus isn't moved on view changes.
**Fix:** Settings/profile panel + CSS-variable theme system respecting `prefers-color-scheme`; wrap login in `<form onSubmit>` with `type=submit`, autofocus username, `inert` the hidden IP field, move focus on navigation.

---

## 7. Feature Roadmap

Most of these have **partial DB or Rust scaffolding already** and need completing + surfacing.

| Feature | Severity | Scaffolding present | Effort |
|---|---|---|---|
| Real presence / online status | critical | `update_user_online_status` (`lib.rs:74`), `UserInfo`/`UserList`, `room_clients` | M |
| Live room member list / sidebar | critical | `room_clients`, `get_users` (unused), `UserInfo` | M |
| Persistent identity + auth | critical | `get_user_by_id` (unused), `localStorage` userId | L |
| User-created rooms/channels | critical | `created_by`/`is_private` columns (never written) | M |
| Connection-status UI + logout teardown | high | disconnect cmds exist (see 1.1/1.2) | M |
| Message history pagination (50-msg ceiling) | high | `get_room_messages` LIMIT 50 | M |
| Message edit & delete | high | needs `message_id` + `MessageType::Edit/Delete` | M |
| @mentions + desktop notifications | high | needs `tauri-plugin-notification` | M |
| Unread tracking + per-room badges | high | needs `last_read_at`, multi-room listener (5.1) | M |
| Direct/private messages | high | `is_private` plumbed but ungated | L |
| Working server discovery in UI | high | `discover_servers` built, unused, no handshake | M |
| Typing indicators | medium | needs `MessageType::Typing` | S |
| Emoji reactions | medium | `is_emoji` infra (cosmetic today) | M |
| Replies / threads | medium | needs `parent_message_id` | L |
| File / image sharing | medium | `tauri-plugin-dialog` present, 10MB frame cap | L |
| Message search | medium | needs FTS5 / LIKE + command | M |
| User profiles & avatars | medium | users table has no avatar/status | M |
| Settings/preferences persistence | medium | only `nutler.userId` persisted | S |
| Admin / moderation | low | `created_by` never set, no roles | L |
| Offline outbox / message queue | low | needs `message_id` + status (5.6) | L |
| Multi-window support | low | single hardcoded window | M |

Notable dependency: **most high-value features depend on the stable `message_id` (1.5/4.1) and the multi-room listener refactor (5.1)** — do those first.

---

## 8. Engineering / Testing / CI

### 8.1 Zero automated tests *(critical)*
No `#[test]`/`mod tests` in `src-tauri/src`; no Vitest/RTL; no `test` script.
**Fix:** Rust `#[cfg(test)]` for frame encode/decode (round-trip, truncated, zero-length) and `MessageType` serde incl. `RoomLeave`; a `tokio::test` integration test that boots `server_listen_as_participant` on a random port, connects a client, sends a chat frame, asserts relay + DB insert (seed an in-memory `sqlx::SqlitePool::connect("sqlite::memory:")` with `migration::get_migrations()`). Frontend: Vitest + RTL + jsdom — LoginView validation/mode toggle, RoomList empty+join, a `useChatConnection` test asserting a null `message` payload doesn't crash.

### 8.2 CI only runs on tags; no lint/typecheck/test gate *(high)*
`.github/workflows/build.yml` triggers only on `push: tags: v*` + `workflow_dispatch`; the single job runs `npm install` + `tauri build` with no `tsc --noEmit`, eslint, `cargo test/clippy/fmt`.
**Fix:** Add `on: pull_request` + `push: branches:[main]`; a fast ubuntu-only `check` job: `npm ci`, `tsc --noEmit`, eslint, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, vitest. Keep the multi-OS bundle gated to tags.

### 8.3 CI installs deprecated `libwebkit2gtk-4.0-dev` — fails on ubuntu-24.04 *(high)*
`build.yml:48` on `ubuntu-latest` (now 24.04) where the 4.0 package is gone; `libappindicator3-dev` also deprecated.
**Fix:** `libwebkit2gtk-4.1-dev` + `libayatana-appindicator3-dev` (Tauri 2 deps), or pin `ubuntu-22.04`.

### 8.4 No Rust caching; pins Node 23.10.0; outdated release action *(medium)*
No `Swatinem/rust-cache`; `node-version: 23.10.0` (non-LTS); `softprops/action-gh-release@v1`; `npm install` not `npm ci`.
**Fix:** Add `Swatinem/rust-cache@v2`; Node `22` LTS (or `.nvmrc`); `npm ci`; `action-gh-release@v2`.

### 8.5 No ESLint; Prettier present but unwired *(medium)*
No eslint config/dep; `prettier` present with no `lint`/`format` script. Anti-patterns like `key={idx}` and exhaustive-deps issues go uncaught.
**Fix:** eslint + `@typescript-eslint` + `eslint-plugin-react-hooks` + `react`; `lint`/`format` scripts + `.prettierrc`; enforce in CI; consider husky + lint-staged.

### 8.6 Backend logging is emoji `println!`/`eprintln!` — no `tracing` *(medium)*
38 `println!`/`eprintln!` across `src-tauri/src`; no `tracing`/`log`. No level control, no structured fields.
**Fix:** Adopt `tracing` + `tracing_subscriber` in `run()`; replace prints with `info!/warn!/error!` + structured fields (`peer_addr`, `room`, `user_id`); gate via `RUST_LOG`.

### 8.7 Stringly-typed `Result<_, String>` errors *(medium)*
22 occurrences; frontend only `console.error`s, so no machine-readable code to branch on (connection-refused vs db-locked vs duplicate-email).
**Fix:** `thiserror`-based `AppError` enum implementing `serde::Serialize`; `type AppResult<T>`; frontend switches on a stable code to drive toasts. (Unlocks 5.3 / 6.3.)

---

## 9. Repo Hygiene & Docs

| Item | Severity | Evidence | Fix |
|---|---|---|---|
| README claims MIT but no LICENSE file | high | no `LICENSE`; no `license` in Cargo.toml | Add MIT `LICENSE` + `license="MIT"` |
| Leftover scaffolding + dead `greet` command | medium | `src/section/starter.css`, `src/assets/react.svg`, `src/constants/index.d.ts` (competes with `types.ts`), `public/vite.svg`/`tauri.svg`, `greet` (`lib.rs:18,68`) | Delete dead files; remove `greet` from handler; set `index.html` title/favicon |
| Placeholder Cargo metadata | medium | `description="A Tauri App"`, `authors=["you"]` | Real description/authors/license/repository |
| No governance docs | low | no CONTRIBUTING/CHANGELOG/SECURITY/templates/dependabot | Add them (SECURITY matters for a plaintext LAN app) |
| `deploy.sh` brittle / diverges from CI | low | `npm install`, wrong bundle path, unsigned | Delete in favor of CI artifacts, or harden (`npm ci`, checksums, codesign) |
| **Stale finding (no action):** `.idea/` and `dist/` are gitignored and NOT tracked | — | `git ls-files` shows neither | Do **not** re-action |

---

## 10. Phased execution plan

### Phase 0 — Stop the bleeding (bug fixes + hygiene) — ✅ DONE (2026-06-30)
- [x] Cap the client read loop; factor a shared `read_frame()` with one `MAX_FRAME` (1.1/2.4) — **S**
- [x] Resolve the two-pool DB race: single pool on `app_config_dir` (matches the plugin), built synchronously via `block_on` in `setup` + FK/WAL/busy_timeout (1.3/4.4) — **M**
- [x] Standardize timestamps end-to-end + single `normalizeMessage()` (UTC-correct) (1.4) — **M**
- [x] Add `message_id` column (migration v8) + thread through save/load/types; `ON CONFLICT DO NOTHING`; dedup + React key by id (1.5/5.7/4.1) — **M**
- [x] Add `ORDER BY created_at DESC, id DESC` tie-break + perf indexes + `UNIQUE(chat_rooms.name)` (1.6/4.2/4.3) — **S**
- [x] Wire `logout` to call `client_disconnect`/`server_participant_disconnect`; replace `emit("message", ())` with typed `disconnected`/`server_stopped` events (1.1/1.2) — **M**
- [x] Fix `joinRoom` to pass real `oldRoom` (1.10) — **S**
- [x] Add `LICENSE`, fix Cargo metadata, set `index.html` title/favicon, delete dead scaffolding + `greet` (9) — **S**
- [x] _Folded-in design/quick wins:_ real app title + favicon, window 1100×720 with min-size (6.8), `prefers-reduced-motion` reset (6.4), explicit `tokio`/`sqlx` features (3.4).

> **Remaining for later phases (deferred from the related findings above):** the full `read_frame`-shares-the-cap is done, but the **broadcast lock/ordering refactor (3.2/3.3)**, **reconnect idempotency (1.7)**, **`RoomLeave` end-to-end (1.8)**, and **stale-cleanup room (1.9)** are Phase 1. The **timestamp wire format is normalized on the frontend**; making the Rust `Message.created_at` a string too (so no normalization is needed at all) is a nice follow-up.

### Phase 1 — Security + reliability foundation _(in progress)_
- [x] **Auth + encryption (2.1/2.2/2.3)** — **shared room password + Noise (NNpsk0) encryption**, end-to-end. `secure.rs` transport (built + unit-tested) is now wired into both handshake sites: responder on accept, initiator on connect; all transport frames are encrypted; a wrong password fails the handshake and the connection is dropped. Login UI has a password field (host sets it, clients enter it); reconnect re-derives the key from a ref (not persisted). _Live two-instance verification still recommended (can't GUI-test here). Per-user identity remains self-asserted among authenticated peers — that's inherent to the shared-password model chosen over per-user tokens._ — **L**
- [ ] Make `Connect` idempotent on the host; cancel old client listener on reconnect (1.7) — **M**
- [ ] Implement `RoomLeave` end-to-end + handle `Disconnect`/`RoomLeave` in server match; fix stale-cleanup room (1.8/1.9) — **M**
- [ ] Refactor broadcast: collect targets, drop locks before emit/spawn, sequential ordered sends, write timeout + heartbeat (3.2/3.3) — **M**
- [~] Bounded accept loop ✅ (2.5); per-IP cap + rate limit + read-timeout still pending (read-timeout pairs with the heartbeat in 3.3) — **M**
- [x] Replace network-path `unwrap`/`expect` with graceful handling; `now_secs()` helper (2.6) — **M**
- [x] Remove `sql:allow-execute` (+ `sql:default` + unused `dialog`); set strict CSP (2.7) — **S**
- [x] Input validation/length caps at command + wire boundary (2.8) — **M**
- [x] DB integrity: `foreign_keys=ON`, WAL, busy_timeout, indexes, `UNIQUE(chat_rooms.name)` (Phase 0), `upsert_user` normalization, `join_room` upsert (4.2-4.6/4.8). _FK `ON DELETE` still pending — needs a table-rebuild migration._ — **M**

### Phase 2 — UX/layout overhaul to a persistent sidebar
- [ ] Persistent 3-pane shell (sidebar + center + members), no full-screen swaps; demote "leave room" (6.1) — **L**
- [ ] Register `message` listener once + per-room message store `Map<roomId, Message[]>` (5.1) — **M**
- [ ] Reconnect state machine via `useReducer`/refs; clear timers; status banner (5.2/6.3) — **M**
- [ ] Toast/error system + inline login errors; typed `AppError` codes (5.3/6.3/8.7) — **M→L**
- [ ] Accessibility pass: aria-labels, `<label>`s, `role="log"`/`aria-live`, focus management, Enter-to-submit (6.2/6.10) — **M**
- [ ] `prefers-reduced-motion`, contrast fixes, window min-size, loading/empty states + scroll-to-bottom (6.4/6.5/6.7/6.8/5.10) — **M**
- [ ] Split god-hook into `useConnection`/`useMessages`/`useAuth` (5.5) — **L**

### Phase 3 — Feature expansion
- [ ] Real presence (`update_user_online_status` on login/logout + presence event) + live member list (7) — **M**
- [ ] Persistent identity + session restore via `get_user_by_id` (7) — **L**
- [ ] User-created rooms (`create_room`, `created_by`/`is_private`, "New Channel" UI) (7) — **M**
- [ ] Unread tracking (`last_read_at` + badges) — depends on multi-room listener (7) — **M**
- [ ] @mentions + desktop notifications (`tauri-plugin-notification`) (7) — **M**
- [ ] Message edit/delete (`edited_at`/`is_deleted` + `MessageType::Edit/Delete`) (7) — **M**
- [ ] History pagination (cursor-based, load-older on scroll-up) (7) — **M**
- [ ] Working discovery in UI (handshake + mDNS/UDP, replace port scan) (7/2.10) — **M**
- [ ] DMs, reactions, typing indicators, search, avatars, settings persistence (7) — **S→L each**

### Phase 4 — Polish, tests, CI, release
- [ ] Rust unit + `tokio::test` integration tests; Vitest + RTL frontend tests (8.1) — **L**
- [ ] CI `check` job on PRs (tsc/eslint/clippy/fmt/test); fix `libwebkit2gtk-4.1`, Node 22, `npm ci`, rust-cache, release action v2 (8.2-8.5) — **M**
- [ ] `tracing` logging (8.6); ESLint/Prettier wired + pre-commit (8.5) — **M**
- [ ] Governance docs (CONTRIBUTING/CHANGELOG/SECURITY/templates/dependabot); harden or delete `deploy.sh`; codesign/notarize (9) — **M**
- [ ] PII at rest: SQLCipher/keychain, file perms (2.11) — **M**

---

## 11. Quick wins (high impact, low effort)

- [ ] **Cap the client read loop** — closes a remote OOM-DoS in a few lines (1.1/2.4) — **S**
- [ ] **Add the `ORDER BY … , id DESC` tie-break** — fixes out-of-order history instantly (1.6) — **S**
- [ ] **Remove `sql:allow-execute` + set a strict CSP** — collapses the XSS→DB-compromise chain (2.7) — **S**
- [ ] **Stop emitting `()` on the `message` channel** — use a typed `disconnected` event (1.2) — **S**
- [ ] **Add `LICENSE`, fix Cargo metadata, set window title/favicon, delete dead scaffolding + `greet`** — instant professionalism (9) — **S**
- [ ] **Add the three missing indexes + `UNIQUE(chat_rooms.name)`** — fixes scans and cross-room leakage (4.2/4.3) — **S**
- [ ] **Declare real `tokio` features** — removes latent build fragility (3.4) — **S**
- [ ] **Fix CI Linux deps to `4.1` + Node 22 + `npm ci`** — unbreaks releases (8.3/8.4) — **S**
- [ ] **Set window min-size + `prefers-reduced-motion` reset** — accessibility/layout for almost free (6.4/6.8) — **S**

---

_Generated from a 7-dimension specialist audit (102 findings) of the `async-gem3pro` branch. Evidence line numbers reference the source at audit time; re-verify before editing._
