# CollecteMoi

White-label Flutter app (Android, iOS, web lab) + Rust server that moves forensic artefacts — Android bugreport, iOS sysdiagnose, suspicious messages — encrypted as **CM01** envelopes to the case key.

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Status

| Milestone | Status |
| --- | --- |
| Envelope + chunked upload + CLI | Done (`CM01`) |
| On-device / lab CM01 seal | Done (native FFI + `/v1/lab/seal` for web) |
| Android bugreport path | Guided flow + ZIP validation; server stores opaque blob (no parsers) |
| iOS sysdiagnose path | Guided flow + validator; Share Extension sources + App Group entitlements |
| Lab admin console + process ctl | Done (`/admin`, `scripts/collectemoictl`) |
| Node/TS desktop client | Done (`clients/node`) |
| Message reports / white-label / production admin | Next |

---

## Features

### Server (`collectemoi-api` + `collectemoi-worker`)

**Transport & crypto**
- HTTPS by default with auto self-signed certs (`data/tls/`) or your own PEMs; optional plain HTTP companion
- **CM01** envelopes (HPKE-X25519 + STREAM ChaCha20-Poly1305 + P-256 device signature)
- Per-device **CMNK** noise wrap around CM01; optional legacy **CMDEV** when enabled in Settings
- Artefact kinds: `android_bugreport`, `ios_sysdiagnose`, `message_report`

**Enrollment**
- Email OTP enroll (work email must match a company domain; 6-digit / 10 min TTL)
- Lab shortcut keys (`LAB-DEMO01` …) skip the mailbox; seeded reusable tokens (`demo`, …)
- Device P-256 public key + noise key exchange; banned keys rejected on enroll and upload
- Soft tenant manifest + open collection requests; enrollment audit log

**Uploads & storage**
- Chunked upload protocol (create → chunks with SHA-256 digests → complete)
- Sharded staging under `data/uploads/`; parallel clients with per-upload locks
- Worker decrypts completed uploads into **opaque** blobs under `data/blobs/` (no bugreport/sysdiagnose parsers)
- SQLite catalogue at `data/collectemoi.db` (devices, companies, lab keys, uploads, jobs, blobs, access + enrollment audits, admin auth)
- Legacy JSON/JSONL imported once on first open

**Admin console** ([`/admin`](https://127.0.0.1:8080/admin))
- First-run password setup; HttpOnly cookie sessions
- Optional MFA: TOTP and/or WebAuthn (YubiKey); change password; host-local recovery via `collectemoictl reset-password`
- **Overview** — health, KPIs, pipeline pulse, API traffic, attention, recent enrollment, runtime
- **Pipeline** — uploads and decrypt jobs with company / status filters
- **Blobs** — list, metadata, streamed download, delete; download access summary
- **Devices** — list, ban/unban
- **API access** — traffic audit with kind / remote / status / path filters, clickable bars, blob links, and a copyable event sample
- **Companies** — domains, per-company label color, devices, lab keys, tokens, branding/builds; pause blocks enroll
- **Enrollment** — folded into Companies (tokens, lab keys, audit)
- **Keys & bans** — ban client public keys
- **Logs** — live API / worker / connection output (follow, filter, download)
- **Client apps** — per-company, multi-version branding (name, text, colors, logo), live enroll and launcher previews, select the uploaded logo or icon as the launcher image, adjust its size and rotation, and customize optional icon text (font, size, color, badge, opacity, draggable X/Y placement), generate/stop APK/IPA with live build steps, download/delete artefacts (version + SHA-256), delete branding versions (keeps ≥1; optionally wipes matching `data/mobile/<company>/<version>/`)
- **Settings** — display name, public URL, lab seal, allow CMDEV (persisted across restart)
- **Security** — password, TOTP, YubiKey registration
- **Backup** — export/import zip (`collectemoi.db` + optional keys/blobs)

**Abuse / DDoS shields (application layer)**
- Per-IP rolling rate limits (stricter for auth, enroll, lab seal; higher for upload chunks)
- Automatic short cool-down after sustained rate-limit hits
- Global concurrency cap (`--max-concurrent`) and per-request timeout (`--request-timeout-secs`)
- Existing body-size limit + admin auth fail lockout; `/healthz` exempt from rate limits
- Does **not** replace edge DDoS protection (CDN / reverse proxy / firewall) — lab process shield only

**Lab helpers**
- `POST /v1/lab/seal` — server-side CM01 seal for web / desktop when native FFI is unavailable
- Blob list/get/content also on `/v1/blobs…` for lab convenience
- Access audit middleware (optional `X-Device-Id` from clients)

**Process control** (`scripts/collectemoictl`)
- `start` / `stop` / `restart` / `status` / `health` / `doctor` / `logs` / `open` / `version`
- `reset-password` (clears MFA by default; `--keep-mfa` / `--force-setup`)
- `export` / `import` backup (live API or offline)
- Builds binaries when sources (including embedded `admin.html`) change; frees orphan listeners on bind port

---

### Clients

**Flutter app** (`app/` — Android, iOS, web lab)
- White-label tenant bundle (`tenant/default/`) — display name, server URL, collectors, support/privacy links
- Client UI supports English, French, and Spanish, selected from the device/browser language; unsupported languages fall back to the tenant `default_locale`, then English
- Enroll: email OTP, lab key, or QR scan of lab key; Server URL field or `--dart-define=COLLECTEMOI_SERVER_URL=…`
- Soft device keys + noise key; durable local session
- Collectors:
  - **Android bugreport** — guided flow, Developer Options deep links, ZIP/dumpstate validation
  - **iOS sysdiagnose** — guided flow, name/gzip validation, Files import; Share Extension sources ready
  - **Suspicious message** — channel/sender/body/note/flags, URL detect, optional image → CM01 upload
  - **Lab file upload** — arbitrary file through the same seal + chunk pipeline (web/desktop)
- Consent screen before upload; on-disk payload store; upload queue with retry / discard
- **My data** — local job status and retry controls, plus authenticated server-side submission history and clear local enrollment
- Crypto: native FFI to `libcollectemoi_envelope` when present; PointyCastle + `/v1/lab/seal` on web
- Lab builds via `scripts/collectemoi-app` → `dist/mobile` + `data/mobile` (admin download)

**Node / TypeScript desktop** (`clients/node/`)
- CLI: `enroll` (email / lab key / token), `requests`, `upload`, `collect`
- Library: health, manifest, enroll, labSeal, chunked envelope upload
- Local desktop UI (`scripts/collectemoi-client-ui` → `http://127.0.0.1:8090/`) — enroll, requests, seal & upload, session clear
- `--insecure` / `--tls-ca` for lab self-signed certs

**Rust CLI** (`collectemoi-cli`)
- `gen-case-key` / `gen-device-key`
- `seal` / `open` CM01 locally
- `upload` — seal + chunked upload to the API

**Mobile build tooling** (`scripts/collectemoi-app`)
- `apk` / `ios` / `run` / `install` / `devices` / `emulators` / `doctor` / `clean` / `open`
- Debug / profile / release; bake server URL; simulator or device iOS
- Branding: `--tenant-dir`, `--display-name`, `--logo`, `--icon`, `--icon-scale`, `--icon-rotation`, `--icon-text` and its font/size/position/color/badge options (synced into Flutter tenant + launcher labels/icons; image compositing uses Swift/AppKit on macOS)
- Publishes artefacts + `.meta.json` (version, SHA-256) for the admin **Client apps** panel
- Admin can also trigger builds via **Client apps** (select company → Generate new build)

---

### Lab limits (intentional today)

- Open requests and tenant collectors are demo/hardcoded — not analyst-authored case workflows yet
- Worker stores opaque blobs only (no real bugreport/sysdiagnose analysers)
- `POST /v1/messages` is an accept stub; message collection in the Flutter app uses the CM01 upload path
- Architecture roadmap items (Postgres, DPoP, OIDC, KMS, crypto-shred retention UI) are spec-only — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## Prerequisites

```bash
export PATH="/opt/homebrew/share/flutter/bin:$PATH"   # avoid broken ~/dev/flutter
```

Rust stable and Flutter stable.

Configuration uses **CLI flags only** (no `COLLECTEMOI_*` environment variables).

---

## Quick lab (API + worker)

```bash
# One-shot process control (builds if needed) — HTTPS by default
./scripts/collectemoictl start
./scripts/collectemoictl status          # processes, TLS, storage, catalogue
./scripts/collectemoictl health          # exit 0/1 for scripts
./scripts/collectemoictl doctor          # local diagnostics
./scripts/collectemoictl logs -f         # follow api+worker logs
# ./scripts/collectemoictl restart --bind 0.0.0.0:8080 --public-url https://192.168.1.10:8080
# Custom certs (otherwise auto-generated under data/tls/):
# ./scripts/collectemoictl start --tls-cert ./fullchain.pem --tls-key ./privkey.pem
# Optional plain HTTP companion (off by default):
# ./scripts/collectemoictl start --http-bind 127.0.0.1:8081
# ./scripts/collectemoictl stop

# Forgot admin password (host-local — no old password needed):
# ./scripts/collectemoictl reset-password
# ./scripts/collectemoictl reset-password --password 'new-secret' --yes
# ./scripts/collectemoictl reset-password --keep-mfa --yes   # password only; leave TOTP/YubiKey
# ./scripts/collectemoictl reset-password --force-setup --yes
```

Or manually:

```bash
cargo build -p collectemoi-envelope
cargo run -p collectemoi-api -- \
  --data-dir ./data \
  --bind 127.0.0.1:8080 \
  --public-url https://127.0.0.1:8080 \
  --lab-seal true
cargo run -p collectemoi-worker -- --data-dir ./data
```

**TLS / HTTPS only by default.** If `--tls-cert` / `--tls-key` are omitted, a self-signed cert is written to `data/tls/server.{crt,key}` (SAN: localhost, 127.0.0.1, host from `--public-url`). Pass your own PEMs for production. Plain HTTP is off unless you set `--http-bind` (or `--tls false` for lab-only plaintext).

### Docker package and quick VPS deploy

The root `Dockerfile` builds a small runtime image containing both the API and worker. The API listens on HTTPS port 8080 and stores its database, generated TLS certificate, uploads, blobs, and keys under `/data`.

To create a portable image archive locally (Docker Buildx required):

```bash
./scripts/collectemoictl package --platform linux/amd64
# Use linux/arm64 for an ARM VPS; optional: --out ./collectemoi.tar.gz --tag collectemoi:try1
```

For a quick test deployment, make sure the VPS has Docker installed and your SSH key can log in and run Docker, then run:

```bash
./scripts/collectemoictl push deploy@example.com --public-url https://example.com:8080
# Optional: --ssh-port 2222 --port 8443 --platform linux/arm64
```

`push` packages the image, copies it over SSH, loads it on the VPS, and starts `collectemoi-test-api` plus `collectemoi-test-worker`. The API is published on the selected host port (8080 by default); its admin UI is at `/admin`. Use a DNS name or IP in `--public-url` that resolves to the VPS and is reachable by clients. The server creates a self-signed certificate for that URL, so clients will need to trust it for testing. The containers share the persistent Docker volume `collectemoi-test-data`; rerunning `push` replaces these test containers while retaining their state. Remove that volume explicitly with `docker volume rm collectemoi-test-data` when you want to discard the test server data.

To run the image yourself, mount a persistent volume at `/data` and pass the normal API flags. Override the entrypoint with `/usr/local/bin/collectemoi-worker` to run the worker from the same image.

**Admin console**: [https://127.0.0.1:8080/admin](https://127.0.0.1:8080/admin) — browser warns on the self-signed cert; use Advanced → Proceed (or trust `data/tls/server.crt`).

Clients generate a unique ChaCha20-Poly1305 **noise key** at enroll, exchange it with the server, and wrap CM01 uploads as `CMNK` blobs. Banned device public keys are rejected on enroll and upload complete.

**Email OTP enroll**: `POST /v1/enroll/start` (work email must match a company domain) → 6-digit code (10 min TTL) → `POST /v1/enroll/confirm` with code + device/noise keys. Lab shortcut keys (`LAB-DEMO01` …) skip the mailbox. Admin console lists companies, lab keys, and the enrollment log.

**Persistence**: all metadata/config/audit lives in SQLite at `data/collectemoi.db` (devices, companies, lab keys, uploads, jobs, blobs catalogue, access + enrollment logs). Queries are parameterized; path ids must be UUIDs. Large ciphertext/plaintext files stay under `data/uploads/` and `data/blobs/` (case TLS keys remain under `data/keys/` and `data/tls/`). Legacy JSON/JSONL files are imported once on first open.

**Backup / restore** (zip with `collectemoi.db` + optional keys/blobs):

```bash
./scripts/collectemoictl export --out ./backup.zip --keys
./scripts/collectemoictl import ./backup.zip --force --keys
# Or Admin → Settings → Backup (https://127.0.0.1:8080/admin#settings)
# API: GET /v1/admin/export?keys=true&blobs=false
#      POST /v1/admin/import?confirm=true  (multipart field `file`)
```

### Desktop TypeScript client

```bash
./scripts/collectemoi-client-ui
# → listens on http://127.0.0.1:8090/ (does not open a browser; pass --open to launch one)
# talks to https://127.0.0.1:8080 with --insecure

# Or CLI (lab self-signed):
cd clients/node && npm install
npx tsx src/cli.ts enroll --server https://127.0.0.1:8080 --lab-key LAB-DEMO01 --insecure
npx tsx src/cli.ts enroll --server https://127.0.0.1:8080 --email you@example.com --insecure
npx tsx src/cli.ts collect --file ~/Desktop/sample.zip --artefact android_bugreport --insecure
```

See [clients/node/README.md](clients/node/README.md).

### Flutter web / mobile lab

```bash
cd app && flutter run -d chrome   # or a device
```

1. Enroll with token **`demo`** (set Server URL if needed).  
2. **Lab file upload** → pick a bugreport ZIP (or any test file).  
3. Consent → upload.  
4. Worker decrypts CM01 and stores an opaque blob under `data/blobs/` (no bugreport/sysdiagnose parsers).

### `collectemoi-api` flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--data-dir` | `./data` | Uploads, keys, jobs |
| `--bind` | `127.0.0.1:8080` | Listen address |
| `--public-url` | `https://127.0.0.1:8080` | Advertised in enroll / manifest |
| `--tls` | `true` | HTTPS; auto-generates `<data-dir>/tls/server.{crt,key}` |
| `--tls-cert` / `--tls-key` | `<data-dir>/tls/server.*` | Custom PEM certificate / private key |
| `--http-bind` | `off` | Optional plain HTTP companion (`off` = HTTPS only) |
| `--lab-seal` | `true` | Enable `POST /v1/lab/seal` for Flutter web |
| `--lab-reveal-otp` | `true` | Include OTP in `/v1/enroll/start` responses (lab) |
| `--allow-dev-envelope` | `false` | Accept legacy `CMDEV` blobs |
| `--rate-limit` | `true` | Per-IP rate limits + abuse cool-down |
| `--max-concurrent` | `256` | Max in-flight HTTP requests |
| `--request-timeout-secs` | `120` | Per-request timeout (slowloris / hung clients) |
| `--rate-limit-general` | `180` | General reqs / IP / minute |
| `--rate-limit-auth` | `20` | Auth reqs / IP / minute |
| `--rate-limit-enroll` | `30` | Enroll reqs / IP / minute |
| `--case-pk` / `--case-sk` | `<data-dir>/keys/case.*.hex` | Optional case key files (hex) |
| `--log` | `collectemoi_api=info` | Tracing filter |

```bash
cargo run -p collectemoi-api -- --help
```

### `collectemoi-worker` flags

Decrypts completed uploads into opaque blobs. **Never** runs bugreport/sysdiagnose parsers.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--data-dir` | `./data` | Same tree as the API |
| `--poll-secs` | `2` | Job poll interval |
| `--workers` | `4` | Max parallel decrypt jobs |
| `--case-pk` / `--case-sk` | `<data-dir>/keys/case.*.hex` | Optional case key files |
| `--log` | `collectemoi_worker=info` | Tracing filter |

### Blob API

| Method | Path | Meaning |
| --- | --- | --- |
| `GET` | `/v1/admin/blobs?limit=&offset=&case_id=&artefact=` | List blobs (paginated) |
| `GET` | `/v1/admin/blobs/{id}` | Blob metadata |
| `GET` | `/v1/admin/blobs/{id}/content` | Download raw bytes (streamed) |
| `DELETE` | `/v1/admin/blobs/{id}` | Delete blob |
| `GET` | `/v1/blobs…` | Same list/get/content (lab convenience) |
| `GET` | `/v1/admin/access-stats` | Aggregated API access stats (by kind / IP / blob) |
| `GET` | `/v1/admin/access-log` | Recent API access events (`?kind=&remote=&limit=`) |
| `POST` | `/v1/client/submissions` | Enrolled device submission history (requires an encrypted proof using its registered noise key) |

Uploads are sharded under `data/uploads/<aa>/<bb>/<uuid>/` with per-upload locks so many clients can upload in parallel. API hits are audited to `data/access_log.jsonl` (clients may send `X-Device-Id`).

```bash
cargo run -p collectemoi-worker -- --help
```

---

## CM01 sealing

- **Native (macOS/Linux desktop, future Android `.so`)**: Dart FFI loads `target/debug/libcollectemoi_envelope.dylib`.
- **Web / missing dylib**: `POST /v1/lab/seal` (API `--lab-seal true`).
- Device keys: P-256 from FFI, or PointyCastle on web.
- Case keys: under `data/keys/case.{pk,sk}.hex` (or `--case-pk` / `--case-sk`).

CLI:

```bash
cargo run -p collectemoi-cli -- gen-case-key --out /tmp/case
cargo run -p collectemoi-cli -- gen-device-key --out /tmp/device
cargo run -p collectemoi-cli -- upload --input file.zip \
  --server http://127.0.0.1:8080 \
  --case-id 00000000-0000-4000-8000-000000000001 \
  --case-pk /tmp/case.pk --device-sk /tmp/device.sk
```

---

## Android / iOS

Lab builds (APK / iOS Simulator) via control script:

```bash
export PATH="/opt/homebrew/share/flutter/bin:$PATH"

./scripts/collectemoi-app doctor          # toolchain check
./scripts/collectemoi-app devices
./scripts/collectemoi-app emulators       # list; --launch <id> to boot one

# Debug APK → dist/mobile (+ data/mobile for admin download)
./scripts/collectemoi-app apk --install
./scripts/collectemoi-app apk --server https://192.168.1.10:8080 --install

# iOS Simulator → Runner.app + .zip/.ipa for admin console download
./scripts/collectemoi-app ios --simulator
./scripts/collectemoi-app run --ios       # or --android / --device <id>

./scripts/collectemoi-app open            # reveal dist/mobile in Finder
# Admin UI → Client apps  (brand · generate · download APK / IPA)
```

Or manually:

```bash
cd app
flutter run -d <device>
```

On the enroll screen, set **Server URL** (e.g. `https://192.168.1.10:8080` on a physical phone), or bake it in with `--server` / `--dart-define=COLLECTEMOI_SERVER_URL=…`.  
Bind the API accordingly:

```bash
./scripts/collectemoictl start --bind 0.0.0.0:8080 --public-url https://192.168.1.10:8080
```

- **Android**: share target + Developer Options deep links; client ZIP validation.
- **iOS**: guide + Files import; App Group on Runner.  
  Share Extension: see [`app/ios/ShareExtension/README.md`](app/ios/ShareExtension/README.md).

---

## Tests

```bash
cargo test --workspace
# Regenerate envelope golden vectors (destructive):
cargo test -p collectemoi-envelope regenerate_golden_envelope -- --ignored

cd app && flutter test && flutter analyze
```

---

## Layout

```text
crates/collectemoi-envelope/   # CM01 + C FFI (cdylib)
crates/collectemoi-db/         # SQLite catalogue (parameterized queries)
crates/collectemoi-api/        # enroll, uploads, lab seal, admin console
crates/collectemoi-worker/     # decrypt → opaque blob store (no parsers)
crates/collectemoi-cli/
clients/node/                  # TypeScript desktop client + UI
scripts/collectemoictl         # start|stop|restart|status (API + worker)
scripts/collectemoi-client-ui  # start desktop client UI
scripts/collectemoi-app        # Flutter Android APK / iOS lab builds
app/                           # Flutter (android, ios, web)
data/collectemoi.db            # SQLite: devices, config, uploads, jobs, blobs, audits
data/keys/                     # case keypair (gitignored)
data/tls/                      # auto-generated or custom TLS PEMs
data/uploads/                  # chunked ciphertext staging (sharded)
data/blobs/                    # opaque decrypted artefacts (bytes on disk)
data/run/                      # ctl PID + logs
```
