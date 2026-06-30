# Changelog

All notable changes to Nutler are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-06-30

### Fixed

- **Direct messages across machines.** Clients keep their own local database, so
  host-created DMs and private channels were invisible to them: opening a DM hit
  a foreign-key error and recipients never saw the conversation. Rooms are now
  host-authoritative for clients — the host pushes each client its real room list
  on connect and on every membership change, and clients no longer write room
  membership to their local database.
- **Own messages mis-attributed.** Testing with the same display name on multiple
  machines marked every message as your own (all right-aligned). Ownership is now
  matched by the host-assigned canonical id, so same-named users stay distinct.
- **Reactions counted twice** per click (a duplicated event listener under React
  StrictMode); the reconnect listener had the same latent leak.

### Changed

- Your own messages render right-aligned with a tinted bubble to distinguish them.

## [0.2.0] - 2026-06-30

### Added

- **Direct messages (1:1 and group).** Start a private conversation with one or
  more people from the directory. 1:1 DMs reuse the same room on reopen; group
  DMs are distinct. Conversations are labelled by their other participants and
  live in a dedicated "Direct Messages" section of the sidebar.
- **Invite people to private channels.** A searchable member picker on private
  channels, backed by a host-pushed user directory.
- **App icon** across all platforms.

### Security

- **Membership-gated chat delivery.** Incoming chat frames are now authorized
  against the connection's canonical identity before a message is persisted or
  distributed, so a crafted frame can't inject into a private channel or DM
  history. (Mirrors the existing join/history gates.)

### Changed

- **Canonical identity.** The host is the single authority for user identity
  (keyed by email), fixing cross-machine id collisions that could drop a third
  participant's message delivery.
- **Reconnect generation counter** suppresses a stale listener's
  `connection_lost` once a newer connection exists, preventing spurious
  reconnect loops.

## [0.1.x] — initial release line

### Added

- **End-to-end encryption.** All peer traffic now runs over the Noise protocol
  (`NNpsk0`), with the pre-shared key derived from a shared room password. A wrong
  password fails the handshake.
- **Real-time presence** with a live members panel per room.
- **Message history pagination** — load older messages on scroll-up with viewport
  anchoring.
- **User-created channels**, including private channels.
- **Message edit & delete**, authorship-checked, with inline editing and
  tombstones for deleted messages.
- **@mentions** with highlighting and desktop notifications (on mention or while
  unfocused).
- **Full-text message search** with jump-to-channel.
- **Emoji reactions** with host-authoritative, atomic toggling.
- **Client history sync.** The host pushes a room's recent messages and reactions
  to a joining client over the encrypted socket, so remote clients get real
  scrollback instead of live-only data.
- **Live typing indicators** — ephemeral, encrypted, and never persisted.
- **Session restore** — the login form pre-fills the last-used username, email,
  department, mode, and server address (the room password is never stored).
- **Light/dark theme** toggle (OS-aware, persisted).
- Accessibility pass: keyboard navigation, ARIA roles/live regions, and
  reduced-motion support.

### Changed

- Rebuilt the UI as a three-pane, high-contrast workspace (sidebar, chat,
  members) replacing the previous single-view layout.
- Hardened the networking layer: heartbeat keepalive and read timeouts for
  half-open detection, per-IP connection caps, idempotent reconnect, and
  authoritative stale-connection cleanup.
- Per-connection authorization: edits, deletes, reactions, and room moves are
  authorized against the connection-bound user id rather than the id in a frame.
- Database integrity: foreign keys with `ON DELETE CASCADE`, WAL journaling, and
  a busy timeout.
- Minimum supported Rust version is now **1.89**.

### Fixed

- Numerous concurrency and correctness bugs surfaced by adversarial review,
  including a reconnect race, edit/delete authorship spoofing, reaction
  count-convergence races, pagination cursor mismatches, a history-sync frame
  overflow that could strand a client's loading state, and per-room loading
  state isolation.

### Security

- Added a transport security model and threat boundaries; see
  [SECURITY.md](./SECURITY.md).

