# CollecteMoi Node / TypeScript client

Desktop client that enrolls against `collectemoi-api`, lists collection requests, seals a file via `/v1/lab/seal`, and uploads it with the chunked protocol.

## Setup

```bash
cd clients/node
npm install
```

Requires Node 20+ and a running API with `--lab-seal true` and TLS (defaults).

## Client UI

From the repo root:

```bash
./scripts/collectemoi-client-ui
# UI at http://127.0.0.1:8090/ → API https://127.0.0.1:8080 (--insecure for lab cert)
# ./scripts/collectemoi-client-ui --open
```

Or:

```bash
npm run ui -- --server https://127.0.0.1:8080 --port 8090 --insecure
# npm run ui -- --server https://127.0.0.1:8080 --tls-ca ../../data/tls/server.crt
```

Flow in the UI: **Enroll** (email OTP or lab key `LAB-DEMO01`) → **Requests** → **Seal & upload** a local file.

## CLI

All configuration is CLI flags (no env vars).

```bash
# Enroll (token `demo` is reusable in lab)
npx tsx src/cli.ts enroll \
  --server https://127.0.0.1:8080 \
  --token demo \
  --insecure \
  --session ./.collectemoi/session.json

# See what the server asks for
npx tsx src/cli.ts requests --session ./.collectemoi/session.json

# Upload a desktop file (CM01 via lab seal → chunked upload)
npx tsx src/cli.ts upload \
  --session ./.collectemoi/session.json \
  --file ~/Desktop/bugreport.zip \
  --artefact android_bugreport

# List requests, then upload
npx tsx src/cli.ts collect \
  --session ./.collectemoi/session.json \
  --file ~/Desktop/bugreport.zip
```

Session JSON holds device keys — keep it private (`0600`).
