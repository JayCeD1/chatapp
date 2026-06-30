# Contributing to Nutler

Thanks for your interest in improving Nutler — a LAN-first, end-to-end-encrypted
team chat app built with Tauri (Rust) and React. This guide covers how to set up
your environment, the checks your change must pass, and how to propose it.

## Prerequisites

- **Node.js 20+** and npm
- **Rust 1.89 or newer** (`rustup update stable`) — the notification plugin sets
  this MSRV; older toolchains will fail to build
- **Git**
- Platform build dependencies for Tauri:
  - **Linux:** `libgtk-3-dev libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Windows:** the WebView2 runtime (preinstalled on Windows 11) and the MSVC build tools

See the [Tauri prerequisites guide](https://tauri.app/start/prerequisites/) for details.

## Getting started

```bash
git clone <your-fork-url>
cd nutler
npm install
npm run tauri dev      # launches the desktop app with hot reload
```

To exercise the host/client flow on one machine, run two `tauri dev` instances:
host one with **Host Server** mode, join from the other with **Join Server** and
`127.0.0.1:3625` plus the same room password.

## Project layout

```
src/                     React 19 + TypeScript frontend
  components/            UI (Sidebar, ChatPane, Workspace, LoginView, …)
  hooks/useChatConnection.ts   the connection/state hook (most logic lives here)
  session.ts, utils.ts, types.ts
src-tauri/src/           Rust backend
  lib.rs                 app setup + Tauri command registration
  sockets.rs             TCP framing, host/client relay, message handling
  secure.rs              Noise (NNpsk0) transport
  db_queries.rs          SQLite (sqlx) queries + Tauri commands
  migration.rs           schema migrations (append-only)
```

## Before you open a PR

Run the same checks CI runs. All of these must be clean:

**Frontend**

```bash
npm run typecheck      # tsc --noEmit
npm run lint           # eslint
npm run format:check   # prettier --check
npm test               # vitest run
npm run build          # production build must succeed
```

`npm run format` rewrites files with Prettier if `format:check` fails;
`npm run test:watch` runs Vitest in watch mode while developing.

**Backend** (from `src-tauri/`)

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo fmt --all` applies formatting. Keep clippy clean — prefer fixing the lint
over `#[allow(...)]`, and when an allow is genuinely warranted, scope it as
narrowly as possible with a comment explaining why.

## Coding guidelines

- **Match the surrounding code.** Mirror existing naming, comment density, and
  idioms rather than introducing a new style.
- **Database migrations are append-only.** Add a new numbered migration in
  `migration.rs`; never edit a shipped one.
- **Never hold a `Mutex`/`RwLock` guard across an `.await`** in `sockets.rs` —
  snapshot what you need, drop the guard, then do I/O. This rule has prevented
  several deadlocks; keep it.
- **Authorize with the connection-bound `auth_user_id`,** never the user id
  carried in a frame, for anything that mutates or targets a user.
- **Don't persist secrets.** The room password is the encryption PSK; it must
  never be written to disk, logs, or `localStorage`.
- New networked behavior should be considered against the threat model in
  [SECURITY.md](./SECURITY.md).

## Commit & PR conventions

- Write focused commits with a clear, imperative summary line.
- Keep PRs scoped to one logical change; describe what and why, and how you
  tested it (a two-instance smoke test for anything touching networking).
- Reference any related issue. Update `CHANGELOG.md` under `[Unreleased]` for
  user-facing changes.

## Reporting bugs & security issues

- **Bugs / features:** open a GitHub issue with steps to reproduce, expected vs.
  actual behavior, and your OS.
- **Security vulnerabilities:** do **not** open a public issue — follow
  [SECURITY.md](./SECURITY.md).

By contributing, you agree your contributions are licensed under the project's
[MIT License](./LICENSE).
