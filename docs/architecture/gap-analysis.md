# Nutler vs the Messaging-Platform Requirements — Gap Analysis

A constraint-by-constraint reading of **where Nutler stands today** against
[messaging-platform.md](./messaging-platform.md), so we can tread carefully and avoid
decisions that paint us into a corner. Line references are indicative; re-verify before
editing.

> **Bottom line.** Nutler is in good shape for a readiness phase. The *transport*-security
> story is already strong (Noise replaces TLS). The biggest deliberate gaps are: (1) the
> message **envelope is unversioned** and mixes metadata with the plaintext body, (2) several
> shipped features **require the server to read plaintext** (search, history sync, body
> persistence) and are therefore **E2EE-incompatible**, and (3) **auth is fused with
> encryption** (the room password *is* the Noise PSK). None of these block current work — but
> each should be a conscious, documented choice, and a couple are nearly-free to fix now.

---

## Scorecard

| # | Constraint | Status | Notes |
| --- | --- | --- | --- |
| 1 | Payloads opaque to server | 🔴 Not met | Host persists, searches, and relays plaintext bodies. Accepted for now (see "E2EE-incompatible features"). |
| 2 | Metadata separate from content | 🟠 Partial | The `Message` envelope carries metadata, but the body (`message`) is an inline plaintext field, not a separable opaque `payload`. |
| 3 | Client-side key management / device identity | 🔴 Not met | Identity is the user account (email→`user_id`). No device identity, no per-device keypairs. |
| 4 | Encryption abstracted (`IMessageCrypto`) | 🟠 Partial | `secure.rs` is a clean **transport**-crypto seam (Noise), but there is no **message**-level crypto layer separate from transport. |
| 5 | Transport security independent | 🟢 Met | Noise (NNpsk0, ChaCha20-Poly1305) protects every frame; it's a distinct module (`secure.rs`). This is our TLS-equivalent. |
| 6 | No server dependence on plaintext | 🔴 Not met (by design, for now) | Search + history sync + persistence read bodies. Explicitly flagged below. |
| 7 | Versioned message format | 🟢 Met | The `Message` envelope carries `version: u16` (`PROTOCOL_VERSION = 1`), serde-defaulted for forward/back compat (ADR-0004). |
| 8 | Attachments encrypted-ready | ⚪ N/A yet | No attachments feature yet — design it encrypted-blob + metadata-sidecar from day one. |
| 9 | Auth separate from encryption | 🔴 Not met (by design, for now) | The room password derives the Noise PSK **and** is the sole access control. Revisit before per-user auth / E2EE. |
| 10 | Out-of-band key verification | ⚪ N/A yet | No keys to verify yet; required before any E2EE rollout (Constraint 10/11). |
| 11 | Migration path to FS/PCS/groups not blocked | 🟢 OK | Nothing precludes a future Double-Ratchet 1:1 path; group E2EE will need its own model. |
| 12 | Presence/typing decision documented | 🟢 Decided | Treated as **server-visible metadata** (UserList, `MessageType::Typing` relayed by host). Recorded in [decisions.md](./decisions.md). |

Legend: 🟢 met · 🟠 partial · 🔴 gap · ⚪ not applicable yet.

---

## What's already aligned (keep these)

These existing choices match the north star and should be preserved:

- **Client-generated, globally-unique `message_id`** (UUID v4) on every wire `Message` —
  satisfies the Constraint 2 metadata id and the routing-layer "client-generated IDs" rule.
- **Dedup keyed on `message_id`** — both in the DB (`ON CONFLICT(message_id) DO NOTHING`,
  `db_queries.rs`) and the frontend store. This is exactly the routing-layer dedup the spec
  asks for (Rule 10), even though it isn't yet a *dedicated* layer.
- **Routing targets are stable `user_id`s, not IPs, in the upper layers.** The frontend keys
  everything by room name / `user_id`; only the lowest level (`server_streams: user_id →
  TCP write half`) knows about sockets. Upper layers already honor Rule 4.
- **Server-as-relay shape.** The host/client model (`server_listen_as_participant` /
  `client_connect_to_server`) is the right silhouette for an eventual untrusted relay.
- **Transport crypto is a separate module** (`secure.rs`) — a clean seam for Constraint 5.

---

## E2EE-incompatible features (flagged, per Constraint 6)

These ship today and **will not work once the server can't read message bodies.** That's an
acceptable trade for a LAN tool now — but every one must be a conscious entry here, and any
new plaintext-dependent feature must be flagged in its PR (see the PR checklist).

| Feature | Where | Why incompatible | Future path under E2EE |
| --- | --- | --- | --- |
| Full-text message search | `db_queries.rs` `search_messages` (LIKE on `messages.message`) | Server reads plaintext bodies | Client-side index over decrypted local cache |
| Client history sync | `sockets.rs` `send_room_history` (host reads stored bodies and forwards) | Host must read plaintext to build the batch | Server returns ciphertext blobs; client decrypts |
| Message body persistence | `messages.message` column (plaintext) | Server stores readable content | Store opaque `payload` ciphertext |
| Server-side reactions | `reactions` table + host aggregation | Server reads emoji content/links them to messages | Encrypted reactions or client-side aggregation |

> Presence, typing, and read state are **intentionally** metadata (not content) — see
> Constraint 12 / [decisions.md](./decisions.md). They leak activity but not message bodies.

---

## Near-term moves (cheap, do them when adjacent)

You don't need an E2EE sprint. Two moves are nearly free and buy real forward-compat:

1. **Add a `version` field to the message envelope (Constraint 7).** A `u16`/`u8` `v` on the
   `Message` struct (`sockets.rs`) + the frontend `Message` type, defaulting to `1`. Do it
   the next time the format changes — e.g. **alongside the unread-spine `LoadOlder` work** —
   so v1 (plaintext) and a future v2 (ciphertext) can coexist without a flag day.
2. **Keep the body separable in your head as `payload`.** When you next add a field that is
   message *content* (attachments, rich text), put it under a content sub-object, not a new
   top-level sibling of the routing metadata — so a future "encrypt the content object" is a
   localized change.

Deliberately **not** doing yet (would be over-abstraction per Guardrail 1):

- An `IMessageCrypto` trait with a no-op impl — defer until there's a second (real) provider.
  For now `secure.rs` documents the seam.
- An `ITransport` trait — defer until a second transport is real. The seam lives at the TCP
  read/write + framing in `sockets.rs`; when extracting, the routing logic
  (`handle_server_message`, `distribute_message_to_all`, `room_clients`) must move **above**
  it, not with it.
- Device identity / keypairs (Constraint 3) and key-verification UI (Constraint 10) — these
  belong to the E2EE phase; just don't fuse new code to the assumption that identity ==
  account.

---

## The one coupling to watch (Constraint 9)

Today the **room password is overloaded**: it derives the Noise PSK (confidentiality) *and*
is the only access control (anyone with it is a trusted participant). That's a fine model for
a small trusted LAN, and it's documented in [SECURITY.md](../../SECURITY.md). But it means
**auth and encryption are fused**, which Constraint 9 warns against. Before introducing
per-user accounts, sessions, or E2EE, separate these: authentication (who you are) must not be
the same secret as message confidentiality (what can be read). Treat any work that leans
*harder* on "the room password is identity" as adding future migration cost.

---

## Summary of what to change, and when

| Change | Effort | When |
| --- | --- | --- |
| Add `version` to the message envelope | S | With the next format change (unread spine) |
| Keep message content under a `payload`/content sub-object | S | When adding attachments / rich content |
| Document each new plaintext-dependent feature in its PR | — | Ongoing (PR checklist) |
| Separate auth from the encryption PSK | M | Before per-user auth or E2EE |
| Extract `ITransport` + lift routing above transport | L | When a 2nd transport becomes real |
| Device identity + keypairs + verification UI | L | E2EE phase |
