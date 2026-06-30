# Architecture Requirements: Secure, Transport-Agnostic Messaging Platform

**Status:** Future architectural requirements (readiness phase)
**Scope:** Messaging subsystem design constraints for current and future implementation
**Objective:** Build today's LAN chat app in a way that can evolve into true E2EE and
multi-transport communication **without a subsystem rewrite**.

> This is the canonical north-star spec. It intentionally describes a destination we are
> **not** required to reach now. For where Nutler stands against each constraint today and
> what to do next, read the companion [Gap Analysis](./gap-analysis.md). For the record of
> binding choices, see the [Decision Log](./decisions.md).

---

## Executive summary

This document consolidates two related architecture requirements:

1. **End-to-End Encryption (E2EE) readiness** — ensure the server can later be treated as
   an untrusted relay for message content.
2. **Transport-agnostic communication stack** — ensure messaging logic does not depend on
   TCP/IP or any single network technology.

**Current phase:** No full E2EE or alternate transports are required immediately. TLS (and
in Nutler, the Noise transport) and server-side plaintext processing may continue while
features are developed. The goal is to **avoid architectural decisions that make future
migration impractical.**

**North star:** A resilient communication platform that preserves the same application
behavior and security model across LAN, private infrastructure, mesh, VPN, and other future
connectivity — with message confidentiality protected **even if the server is compromised.**

---

## Target layered architecture

```
Application Layer
(Chat UI, Contacts, Presence, Attachments, User Interactions)
        │
Secure Messaging Layer
(E2EE, Encrypt/Decrypt, Signatures, Integrity Verification)
        │
Routing & Synchronization Layer
(Peer Discovery, Message Routing, Queueing, Delivery,
 Offline Sync, Dedup, Ordering, Conflict Resolution)
        │
Transport Adapter Layer
(TCP/IP LAN, Wi-Fi, Ethernet, VPN, Mesh, Radio, Satellite, Future Transports)
        │
Physical Network
(Ethernet, Wi-Fi, Fiber, Radio, Mobile, etc.)
```

### Layer responsibilities

| Layer | Owns | Must NOT do |
| --- | --- | --- |
| Application | UI, conversations, user interactions | Know transport details; perform encryption |
| Secure Messaging | All cryptographic operations | Route messages; open sockets |
| Routing & Sync | Where messages go, retries, offline queue, dedup | Parse plaintext; depend on TCP/IP |
| Transport | Move bytes between nodes | Business logic; encryption |
| Physical network | Connectivity medium | Anything in-app |

- **Application example:** `SendMessage(conversationId, plaintext)` — the app submits
  messages for delivery and does not care how they travel.
- **Transport example:** `ITransport` — `Connect()`, `Disconnect()`, `Send(peerId, bytes)`,
  `Receive(peerId) -> bytes`, plus capabilities (see below).

---

## Part 1 — E2EE readiness requirements

### Goal

The messaging subsystem must be designed so it can later evolve into true end-to-end
encryption, where:

- The server **cannot** decrypt user conversations.
- Message encryption occurs on the **sender's** device.
- Message decryption occurs only on the **recipient's** device(s).
- The server acts only as a **relay and encrypted-message store**.

TLS (and Noise) remain mandatory but are **not** a substitute for E2EE. Transport encryption
and message encryption are separate concerns and must coexist.

### Constraint 1 — Treat message payloads as opaque data

The messaging pipeline must avoid business logic that depends on reading message contents.
Long-term: clients encrypt payloads; the server stores and forwards ciphertext; the server
does not inspect message bodies. **Avoid** coupling routing, persistence, or application
logic to plaintext message content.

### Constraint 2 — Separate metadata from message content

Design storage so metadata and encrypted content are independent.

- **Metadata** (may remain server-visible for routing): `messageId` (client-generated,
  globally unique), `senderId` / `deviceId`, `recipientId(s)` / `conversationId`,
  `timestamp`, delivery status, protocol version.
- **Content** (eventually always ciphertext): message body, attachments, optionally
  encrypted group titles, reactions, etc.

Store content independently from metadata (separate fields, tables, or blobs).

### Constraint 3 — Plan for client-side key management

Anticipate that each device eventually has a long-term public key (registerable with the
server) and a long-term private key (never leaves the device).

**Identity model (required):** distinguish **user identity** (account) from **device
identity** (keypair). One user may have multiple devices, each with its own keypair. Define
behavior for: new-device login, device revoke, logout, key rotation. The server should
ultimately store only public keys (and pre-keys for offline delivery). The server must
**not** own or generate user encryption private keys.

### Constraint 4 — Abstract the encryption layer

Do not embed encryption directly into transport or persistence code. Expose a clear
abstraction:

```
IMessageCrypto
  encrypt(plaintext, context) -> ciphertext
  decrypt(ciphertext, context) -> plaintext
```

**Current phase:** implement a pass-through / no-op provider internally. **Future phase:**
swap in client-side cryptography (libsodium, Signal Protocol, etc.) without changing
application or transport code. Neither the application nor the transport layer encrypts
directly.

### Constraint 5 — Keep transport security independent

TLS/Noise is mandatory for all client–server and inter-node communication where applicable.

- Transport security protects data **in transit** (wire sniffing, on-path MITM).
- E2EE protects message content **from the server** and other untrusted intermediaries.

The future E2EE layer must not replace transport security; it exists **in addition** to it.

### Constraint 6 — Avoid server-side dependencies on plaintext

Do not build features that require the server to read message bodies **unless explicitly
accepting they will not work under E2EE**. Examples to avoid (or flag as E2EE-incompatible):
server-side full-text search, server-side moderation on plaintext, server-side content
indexing, server-side link-preview generation from message text. If such capabilities are
needed later, plan for client-side alternatives or accept metadata-only server features.

### Constraint 7 — Version the message format

Serialization must support protocol versioning so encrypted formats can coexist with legacy
messages. Recommended envelope shape:

```
version        // protocol version
metadata       // routing fields (see Constraint 2)
payload        // opaque bytes (plaintext today, ciphertext later)
attachments    // opaque blobs (encrypted later)
```

### Constraint 8 — Attachment support

Attachments follow the same security model as text: encrypted on sender, stored encrypted on
server, decrypted on recipient. Do not assume files will always be server-readable. Use the
**encrypted blob + metadata sidecar** pattern from day one.

### Constraint 9 — Authentication vs encryption

Keep user authentication separate from message encryption.

| Concern | Purpose |
| --- | --- |
| Authentication | Proves who is logged in (sessions, JWTs, etc.) |
| Encryption | Protects message confidentiality |

Do not tightly couple login/session tokens with message decryption keys.

### Constraint 10 — Trust verification (key fingerprints)

Support out-of-band key verification before or during E2EE rollout: display safety numbers or
QR codes derived from public keys; allow users to verify peer/device keys in person or via a
second channel. Without this, a compromised server could substitute public keys and MITM even
with E2EE enabled.

### Constraint 11 — Future protocol target

The initial E2EE implementation need not match Signal immediately, but the architecture must
not prevent migration toward authenticated public-key exchange, ephemeral session keys,
forward secrecy, post-compromise security, and Double-Ratchet–style session evolution.

**Group chats:** 1:1 and group messaging should share the same opaque-payload contract, but
group E2EE requires a dedicated model (Sender Keys, MLS, or equivalent) and key rotation on
membership changes. Do **not** assume 1:1 crypto extends trivially to groups.

### Constraint 12 — Presence and typing indicators

Decide explicitly whether presence, typing indicators, and read receipts are **metadata**
(server-visible, simpler, leaks activity) or **encrypted control messages** (more private,
more complex). Document the choice and apply it consistently.

---

## Part 2 — Transport-agnostic communication requirements

### Objective

The communication subsystem must be independent of the underlying transport. The application
must not assume messages always travel over traditional TCP/IP LAN. Initial implementation
may use only TCP/IP over LAN — that is **one transport implementation, not an architectural
assumption**.

### Design principles

- **Transport agnostic.** Business logic must not depend on TCP, IP addresses, Ethernet,
  Wi-Fi, or any specific medium. The routing layer may keep a reachability table
  (`nodeId → transport + address`), but chat UI and crypto must not reference
  transport-specific addresses. Use **stable node IDs and device IDs**; transports resolve
  how to reach them.
- **Pluggable transports.** Adding a transport should not change chat UI, message models,
  routing logic, or encryption logic — ideally only a new transport adapter is added.
- **Encryption independent of transport.** Messages are encrypted **before** reaching the
  transport layer; the medium must not affect confidentiality.
- **Routing independent of transport.** Routing operates on abstract nodes/endpoints, never
  assuming every node has an IP or that communication is always TCP.

### Routing layer — additional responsibilities

| Responsibility | Notes |
| --- | --- |
| Message IDs | Client-generated, globally unique |
| Deduplication | Same message may arrive twice over different transports or retries |
| Ordering | Define policy (causal vs total); chat typically tolerates eventual order |
| Offline queue | Store-and-forward when a peer is unreachable |
| Sync on reconnect | Reconcile after a network partition |
| Conflict resolution | Where applicable (e.g. edit conflicts) |

These become critical for mesh, DTN, and multi-transport environments.

### Transport adapter interface

```
ITransport
  connect(peerId) -> Result
  disconnect(peerId) -> void
  send(peerId, bytes) -> DeliveryResult
  receive(peerId) -> stream<bytes>   // or callback-based equivalent
  capabilities() -> TransportCapabilities
```

`TransportCapabilities` should expose at minimum: reliable vs datagram semantics, ordered
delivery (yes/no), `max_mtu` / max payload size, and an optional `latency_hint` for transport
selection. **Pure byte pipes are insufficient for smart routing** — the routing layer needs
reachability and capability info.

### Future capabilities (deferred, but architecture must allow)

Multiple independent LANs; mesh networking; store-and-forward / delay-tolerant networking
(DTN); peer-to-peer direct communication; multi-hop routing; regional synchronization
servers. Not required now — must remain achievable without redesigning the application or
secure-messaging layers.

---

## Part 3 — How the layers interact

```
Application
  SendMessage(conversationId, plaintext)
       │
       ▼
Secure Messaging
  envelope = { version, metadata, ciphertext = encrypt(plaintext) }
       │
       ▼
Routing & Sync
  route by metadata; queue if offline; dedup by messageId
       │
       ▼
Transport
  send(peerId, serialize(envelope))
       │
       ▼
Physical network
```

- **Server role (current):** may process plaintext internally while E2EE is not yet enabled.
- **Server role (future):** stores metadata + opaque payload; cannot decrypt payload.

---

## Part 4 — Implementation priorities

### P0 — Do now (high leverage, low cost)

| Item | Action |
| --- | --- |
| Message schema | `version`, `metadata`, `payload` (opaque bytes), `attachments` |
| Crypto abstraction | `IMessageCrypto` with pass-through implementation |
| Server contract | API and DB never parse `payload` for business logic |
| Message IDs | Client-generated; server deduplicates on `messageId` |

### P1 — Do soon

| Item | Action |
| --- | --- |
| Identity | Stable `nodeId` / `deviceId` separate from IP address |
| Transport | `ITransport` interface + one `TcpLanTransport` implementation |
| Attachments | Encrypted-blob-ready storage (opaque bytes + metadata sidecar) |
| Reachability | Routing table: `nodeId → { transport, address, lastSeen }` |

### P2 — Before E2EE cutover

| Item | Action |
| --- | --- |
| Key registration | Endpoint to store device public keys (unused until E2EE) |
| Verification UI | Safety numbers / QR for key-fingerprint comparison |
| Envelope v2 | Encrypted payload format alongside v1 messages |

### Deferred (E2EE phase and beyond)

Signal Protocol / Double Ratchet; group E2EE (Sender Keys or MLS); alternate transports
(mesh, DTN, satellite); client-side search; sealed sender / advanced metadata privacy.

---

## Part 5 — Testing requirements

| Test type | Purpose |
| --- | --- |
| Message-format golden vectors | Versioned serialization stays stable across releases |
| Crypto-provider swap | Null crypto → real crypto via the same `IMessageCrypto` API |
| Fake transport | Routing, queue, dedup, sync tested without a real network |
| Dedup / reorder | Same `messageId` over two transports delivered once |
| E2EE migration | v1 plaintext and v2 ciphertext messages coexist |

---

## Part 6 — Explicit limitations and tradeoffs

What E2EE does **not** provide:

| Still visible to a compromised server | Protected by E2EE |
| --- | --- |
| Who messages whom | Message body content |
| When messages are sent | Attachment contents |
| Message sizes (approx.) | |
| Online/presence (if metadata) | |
| IP addresses / node reachability | |

**Product implications:** server-side search, spam filters, and link previews require
plaintext or client-side alternatives; group E2EE is significantly harder than 1:1 (plan a
separate crypto path); key verification is a user-facing requirement, not optional hardening.

---

## Part 7 — Scope guardrails

1. **Do not over-abstract prematurely.** Thin interfaces, one working transport first.
2. **Do not block current features on full E2EE** — follow the phased priorities above.
3. **Do not build seven half-finished transports** — prove the stack with TCP/LAN, then add
   adapters.
4. **Do document E2EE-incompatible features** when they are intentionally added (e.g. server
   search) — call it out in the PR description and log it in the
   [Gap Analysis](./gap-analysis.md).

---

## Long-term vision

A resilient communication platform that continues functioning across networking environments,
preserves the same application behavior and UX, maintains message confidentiality even if the
server is compromised, and allows new transports without rewriting chat, crypto, or routing.

**Current implementation:** TCP/IP LAN + Noise + plaintext server processing is acceptable.
**Architectural requirement:** today's code must not make tomorrow's E2EE and multi-transport
goals impractical.

---

## Quick reference — architectural rules

1. Payloads are opaque to the server.
2. Metadata and content are stored separately.
3. Encryption lives only in the Secure Messaging layer.
4. Routing uses node IDs, not IP addresses, in upper layers.
5. Transport moves bytes only.
6. Transport security + E2EE are both required long-term.
7. Messages are versioned.
8. Device identity ≠ user identity.
9. Public keys on server; private keys never leave the device.
10. Dedup, ordering, and offline sync belong in routing — not transport, not UI.

---

_Document version: 1.0 — consolidated E2EE-readiness + transport-agnostic stack requirements._
