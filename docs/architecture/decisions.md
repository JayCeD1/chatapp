# Architecture Decision Log

A lightweight record of binding architectural choices and their consequences, so future work
(and future contributors) understand *why* things are the way they are. Keep entries short.
Add a new one when a decision meaningfully constrains the design; supersede rather than edit.

---

## ADR-0001 — Noise (NNpsk0) for transport security; room password as PSK

**Status:** Accepted (current)

**Context.** Nutler is a LAN-first tool with no central CA or per-user accounts. We need
confidentiality and authenticity on the wire without certificate infrastructure.

**Decision.** Use the Noise protocol in the `NNpsk0` pattern (`secure.rs`), with the
pre-shared key derived as `SHA-256(room password)`. Every frame is encrypted with
ChaCha20-Poly1305; a wrong password fails the handshake.

**Consequences.** Strong transport security with zero PKI. **But** the room password is
overloaded as both the encryption key *and* the access-control credential — auth and
encryption are fused (see [gap-analysis](./gap-analysis.md), Constraint 9). This is fine for a
small trusted LAN and must be revisited before per-user auth or E2EE. Noise is our
TLS-equivalent and satisfies messaging-platform Constraint 5; it is **not** E2EE.

---

## ADR-0002 — Host is the authoritative server today; untrusted relay is the north star

**Status:** Accepted (current), with a stated future direction

**Context.** The host owns the SQLite database and relays messages between clients.

**Decision.** For now the host legitimately reads and stores plaintext (persistence, search,
history sync). The north star ([messaging-platform](./messaging-platform.md)) is a server that
can be treated as an **untrusted relay** of opaque ciphertext.

**Consequences.** Several features depend on a *trusted* server reading bodies and are
explicitly E2EE-incompatible (logged in the gap analysis). New features that require the
server to read message content must be flagged in their PR.

---

## ADR-0003 — Presence, typing, and read state are server-visible metadata

**Status:** Accepted (current)

**Context.** messaging-platform Constraint 12 requires an explicit choice: are presence/typing/
read receipts metadata (server-visible) or encrypted control messages?

**Decision.** Treat them as **metadata.** Presence flows via `UserList`; typing via an
ephemeral `MessageType::Typing` relayed by the host; read state (when built) will be
server-visible `last_read_at`.

**Consequences.** Simpler implementation; leaks *activity* (who is online/typing/when) but not
message *content*. Acceptable under the current trust model. If a future privacy bar requires
hiding activity, these move to encrypted control messages — a deliberate, separate effort.

---

## ADR-0004 — Versioned message envelope

**Status:** Accepted (implemented)

**Context.** The wire `Message` had no protocol-version field (Constraint 7), so a future
ciphertext format couldn't coexist with today's plaintext format without a flag day.

**Decision.** The envelope now carries `version: u16` (`sockets.rs`, `PROTOCOL_VERSION = 1`),
serde-defaulted so frames from pre-versioning / older / newer peers still decode. The frontend
`Message` type carries it through `normalizeMessage` (defaulting to 1). Message **content**
should still migrate under a separable sub-object so "encrypt the content" stays a localized
change (not yet done — payload is still the inline `message` field).

**Consequences.** Cheap insurance now in place: v1 (plaintext) and an eventual v2 (ciphertext)
can coexist, and a peer can branch/reject on `version`. Next step toward E2EE is separating
content from metadata (Constraint 2).

---

## ADR-0005 — Attachments: chunked transfer over Noise, content-addressed SQLCipher blob store, capability-gated

**Status:** Accepted (implemented) — full design in [attachments.md](./attachments.md)

**Context.** File/image sharing (messaging-platform Constraint 8) was the first post-audit
feature. Constraint 8 requires the encrypted-blob + metadata-sidecar pattern *from day one*,
and the gap analysis asked that new message content go under a separable content sub-object
(Constraint 2), so a future "encrypt the content" stays localized.

**Decision.** Four load-bearing choices:
1. **Envelope.** Attachments ride as an optional, serde-defaulted `attachments: Vec<AttachmentRef>`
   content sub-object on `Message` (metadata only — id, sha256, name, mime, size, dims). Blob
   bytes never ride a Chat frame. **No `PROTOCOL_VERSION` bump:** the real coexistence hazard is
   an unknown `MessageType` (an old host drops the connection on parse failure), so the feature
   is gated on a **host capability flag** (`features: ["attachments-v1"]` on the Identity frame),
   not a version bump.
2. **Transfer.** Chunked (44 KiB) over the existing Noise connection — the true per-frame cap is
   Noise's 65 535-byte record, not the 10 MiB app frame. Two-phase upload (blob first, verified
   against its declared sha *before* persisting), pull-based membership-gated download. A second
   blob channel was rejected (duplicate session/auth surface).
3. **Storage** behind a replaceable `blob_store` seam (maintainer requirement): content-addressed
   (sha256) chunked BLOB rows in the existing SQLCipher DB — inherits encryption-at-rest for free
   and removes the path-traversal class. Filesystem/object-storage backends can replace it without
   touching wire or app code. 25 MiB cap.
4. **Opacity.** The host never parses blob bytes (no thumbnailing/indexing/sniffing) — so swapping
   plaintext blobs for ciphertext later is localized.

**Consequences.** Constraint 8 goes from N/A to met; Constraint 2 improves (content is now a
separable sub-object). New host-visible, **E2EE-incompatible** behaviors are logged in the gap
analysis (plaintext-readable blobs, cross-user sha dedup, visible filename/mime metadata). One
open item for a future release gate: sha-based Chat-ref validation currently lets any room member
fetch any stored blob whose hash they learn — either scope validation to shas the sender can
already see, or amend Constraints 6/8 to accept it (attachments.md §9a item 9).
