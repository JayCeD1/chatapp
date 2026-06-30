## What & why

<!-- What does this change do, and why? Link any related issue (e.g. Closes #12). -->

## How it was tested

<!-- Commands run, and a two-instance (host + client) smoke test for anything
     touching networking, encryption, presence, or message delivery. -->

## Checklist

- [ ] `npm run typecheck && npm run lint && npm run format:check && npm test && npm run build` pass
- [ ] `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test` pass (in `src-tauri/`)
- [ ] DB changes use a new append-only migration (no edits to shipped ones)
- [ ] No secrets persisted (the room password is the encryption PSK)
- [ ] `CHANGELOG.md` updated under `[Unreleased]` for user-facing changes
