# Security Policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, use **GitHub's private vulnerability reporting**: go to the repository's
**Security** tab → **Report a vulnerability**. This opens a private advisory
visible only to the maintainers.

Please include:

- A description of the issue and its impact
- Steps to reproduce (a proof of concept if you have one)
- Affected version / commit
- Any suggested remediation

We aim to acknowledge a report within a few days and will keep you updated as we
investigate. Please give us a reasonable window to ship a fix before any public
disclosure.

## Supported versions

Nutler is pre-1.0 and under active development. Security fixes are applied to the
`main` branch and the latest tagged release. There is no backport guarantee for
older tags.

## Security model

Nutler is a **LAN-first** application. Understanding its trust boundaries helps
you report issues that matter and avoid deploying it outside its intended scope.

### What is protected

- **Transport encryption & authentication.** Peers connect over the
  [Noise protocol](https://noiseprotocol.org/) in the `NNpsk0` pattern. The
  pre-shared key is derived (SHA-256) from a **room password** that every
  participant must enter. All traffic is encrypted and authenticated with
  ChaCha20-Poly1305; a wrong password fails the handshake, so peers without the
  password can neither read traffic nor connect.
- **Per-connection authorization.** Each connection is bound to the user id it
  authenticated as. Edits, deletes, reactions, room moves, and history pushes are
  authorized against that bound id — not the id carried in a message — so a peer
  cannot act as, or target, another user by spoofing a frame.
- **Framing hardening.** Length-prefixed frames are bounded to prevent
  oversized-allocation abuse, and the encrypted batch sent on room join is
  trimmed to fit a single Noise message.

### What is *not* in scope

- **Network reachability.** Anyone who can reach the host's TCP port **and** has
  the room password is a trusted participant. There is no per-user identity or
  account system; the room password is the sole access control. Treat it like a
  shared secret and distribute it out of band.
- **At-rest encryption.** Messages are stored in a local SQLite database in the
  OS app-data directory. The database is **not** encrypted at rest; protect the
  host machine accordingly.
- **Internet / hostile-network exposure.** Nutler is designed for trusted local
  networks. Do not expose port 3625 to the public internet.
- **Denial of service** from an authenticated (password-holding) participant is
  out of scope — participants are assumed to be semi-trusted members of the same
  organization.

If you believe something in the "protected" list can be bypassed, that is a
vulnerability we want to hear about.
