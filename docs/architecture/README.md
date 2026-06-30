# Nutler Architecture Docs

These documents describe **where Nutler is headed** and the constraints that keep today's
LAN-chat code from blocking tomorrow's secure, transport-agnostic platform. They are
forward-looking guardrails, not a description of the current implementation alone — read them
before any change that touches the **message format, persistence, transport, routing, or
crypto**.

## Contents

- **[messaging-platform.md](./messaging-platform.md)** — the canonical north-star spec:
  E2EE-readiness and transport-agnostic requirements (the 12 constraints + the layered model
  + phased priorities). This is the destination.
- **[gap-analysis.md](./gap-analysis.md)** — where Nutler stands against each constraint
  **today**, what's already E2EE-incompatible, what's already aligned, and the cheap
  forward-compat moves to make when adjacent.
- **[decisions.md](./decisions.md)** — the architecture decision log (ADRs): the binding
  choices (Noise PSK, server-as-relay, presence-as-metadata, versioned envelope) and their
  consequences.

## How these shape day-to-day work

- The north star is a destination, **not a mandate to build it now.** Follow the phased
  priorities; don't block features on full E2EE; don't over-abstract before there's a second
  real implementation (Guardrails, Part 7).
- The load-bearing rules to honor in *every* change:
  1. Don't deepen the server's dependence on reading plaintext without flagging it.
  2. Keep message **content** separable from routing **metadata**.
  3. Keep encryption in its own seam (`secure.rs` today), not smeared into transport or
     persistence.
  4. Route on stable IDs (`user_id`/room), not transport addresses, above the socket layer.
- When you add a feature that needs the server to read message bodies (search, previews,
  moderation), **say so in the PR** and add it to the E2EE-incompatible table in the gap
  analysis.

See also the project [SECURITY.md](../../SECURITY.md) (current threat model) and
[IMPROVEMENTS.md](../../IMPROVEMENTS.md) (the working roadmap).
