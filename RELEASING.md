# Releasing Nutler

Nutler ships as native desktop bundles built by GitHub Actions. This document
covers the release flow and how to set up **code signing & notarization** so users
don't see "unidentified developer" / SmartScreen warnings.

## Release flow

1. Merge work into `main` via PR (CI must be green — see `.github/workflows/ci.yml`).
2. Bump `version` in `package.json` (the source of truth), then run
   `npm run version:sync` to propagate it to `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml`, and update `CHANGELOG.md`. CI runs `npm run version:check`
   and the release build re-checks it, so a mismatched version can't ship.
3. Tag and push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. The **Build and Release** workflow (`.github/workflows/build.yml`) triggers on
   `v*` tags, builds for every platform in parallel, uploads the bundles as
   artifacts, and attaches them to a GitHub Release.

### What gets built

| Platform | Targets | Bundle |
| --- | --- | --- |
| Windows | `x86_64-pc-windows-msvc` | `.msi` / `.exe` (NSIS) |
| Linux | `x86_64-unknown-linux-gnu` | `.deb`, `.AppImage`, `.rpm` |
| macOS | `x86_64-apple-darwin`, `aarch64-apple-darwin` | `.dmg` / `.app` |

## Signing is optional until you add secrets

The build workflow reads the signing material from repository secrets. **When a
secret is unset, the corresponding build is produced unsigned** — the workflow keeps
working out of the box, it just emits unsigned/un-notarized artifacts. Add the
secrets below to turn signing on; no workflow changes are needed.

Set secrets with the GitHub CLI (or **Settings → Secrets and variables → Actions**):

```bash
gh secret set APPLE_CERTIFICATE < /dev/stdin   # paste/pipe the value
```

---

## macOS — Developer ID signing + notarization

Requires a paid **Apple Developer Program** membership.

### 1. Get a "Developer ID Application" certificate

In Xcode (**Settings → Accounts → Manage Certificates → +**) or the Apple Developer
portal, create a **Developer ID Application** certificate. Export it from Keychain
Access as a `.p12` (select the certificate **and** its private key → Export → set a
password).

### 2. Turn the `.p12` into a secret

```bash
base64 -i Certificates.p12 | pbcopy        # macOS — copies the base64 blob
# then: gh secret set APPLE_CERTIFICATE   (paste)
```

### 3. Create an app-specific password for notarization

At <https://appleid.apple.com> → **Sign-In and Security → App-Specific Passwords**,
generate one for "Nutler notarization".

### 4. Find your signing identity and Team ID

```bash
security find-identity -v -p codesigning   # the full "Developer ID Application: …" string
```
The Team ID is the 10-character code in parentheses (also in the Apple Developer portal
under Membership).

### Secrets

| Secret | Value | Notes |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | base64 of the `.p12` | Tauri imports it into a temp keychain |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password | |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` | exact string from `security find-identity` |
| `APPLE_ID` | your Apple ID email | notarization |
| `APPLE_PASSWORD` | the app-specific password | **not** your Apple ID password |
| `APPLE_TEAM_ID` | your 10-char Team ID | notarization |

Signing turns on when `APPLE_CERTIFICATE` + `APPLE_CERTIFICATE_PASSWORD` +
`APPLE_SIGNING_IDENTITY` are set; notarization additionally needs `APPLE_ID` +
`APPLE_PASSWORD` + `APPLE_TEAM_ID`. Hardened runtime (required for notarization) is
applied via `src-tauri/entitlements.plist`.

> Prefer an App Store Connect **API key** over the Apple ID method? Set
> `APPLE_API_ISSUER`, `APPLE_API_KEY`, and `APPLE_API_KEY_PATH` instead of the three
> `APPLE_ID*`/`APPLE_TEAM_ID` values, and add them to the job `env` in `build.yml`.

---

## Windows — Authenticode signing

The workflow imports a `.pfx` into the runner's certificate store and injects its
thumbprint into `tauri.conf.json` before building (the timestamp URL and digest are
already configured under `bundle.windows`).

### Get a certificate

- **Production:** an OV/EV code-signing certificate from a CA (DigiCert, Sectigo, …).
  EV certificates increasingly require a hardware token and may not be exportable to a
  `.pfx` — for those, prefer **Azure Trusted Signing** (see note below).
- **Testing only:** a self-signed cert (`New-SelfSignedCertificate`) lets you exercise
  the pipeline, but users still get SmartScreen warnings.

### Turn the `.pfx` into a secret

```bash
base64 -i certificate.pfx | tr -d '\n'      # the value for WINDOWS_CERTIFICATE
```

### Secrets

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE` | base64 of the `.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | the `.pfx` password |

> **Azure Trusted Signing** (modern, no exportable cert): configure
> `bundle.windows.trustedSigning` in `tauri.conf.json` and supply the Azure
> credentials as env in `build.yml` instead of the two secrets above. See the Tauri
> Windows signing guide.

---

## Linux

Linux bundles (`.deb` / `.AppImage` / `.rpm`) are not code-signed. AppImages can be
GPG-signed if desired, but it isn't part of this pipeline.

---

## Verifying a signed build

- **macOS:** `codesign -dv --verbose=4 Nutler.app` and
  `spctl -a -vvv -t install Nutler.app` (should report `accepted` / `Notarized
  Developer ID`).
- **Windows:** right-click the `.exe` → **Properties → Digital Signatures**, or
  `Get-AuthenticodeSignature Nutler.exe`.
