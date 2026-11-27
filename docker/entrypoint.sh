#!/bin/sh
# CollecteMoi container entrypoint.
# Usage:
#   docker run … collectemoi:local api [extra api flags…]
#   docker run … collectemoi:local worker [extra worker flags…]
#   docker run … collectemoi:local all    # API + worker in one container
set -eu

DATA_DIR="${COLLECTEMOI_DATA_DIR:-/data}"
BIND="${COLLECTEMOI_BIND:-0.0.0.0:8080}"
PUBLIC_URL="${COLLECTEMOI_PUBLIC_URL:-https://127.0.0.1:8080}"
HTTP_BIND="${COLLECTEMOI_HTTP_BIND:-0.0.0.0:8081}"
LAB_SEAL="${COLLECTEMOI_LAB_SEAL:-true}"
WORKER_POLL="${COLLECTEMOI_WORKER_POLL_SECS:-2}"
RUN_USER="${COLLECTEMOI_USER:-collectemoi}"
export COLLECTEMOI_REPO="${COLLECTEMOI_REPO:-/opt/collectemoi}"
export COLLECTEMOI_DATA_DIR="$DATA_DIR"
export PUB_CACHE="${PUB_CACHE:-$DATA_DIR/.pub-cache}"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$DATA_DIR/.gradle}"
export FLUTTER_HOME="${FLUTTER_HOME:-/opt/flutter}"
export FLUTTER_ROOT="${FLUTTER_ROOT:-$FLUTTER_HOME}"
export ANDROID_HOME="${ANDROID_HOME:-/opt/android-sdk}"
export ANDROID_SDK_ROOT="${ANDROID_SDK_ROOT:-$ANDROID_HOME}"
export PATH="${FLUTTER_HOME}/bin:${ANDROID_SDK_ROOT}/cmdline-tools/latest/bin:${ANDROID_SDK_ROOT}/platform-tools:${PATH}"

# Fresh named volumes are root-owned; fix ownership when we start as root.
ensure_data_dir() {
  mkdir -p "$DATA_DIR" "$PUB_CACHE" "$GRADLE_USER_HOME" "$DATA_DIR/mobile"
  # Seed Flutter pub cache into the persistent volume once.
  seed="${COLLECTEMOI_PUB_CACHE_SEED:-/opt/pub-cache-seed}"
  if [ -d "$seed" ] && [ ! -f "$PUB_CACHE/.collectemoi-seeded" ]; then
    mkdir -p "$PUB_CACHE"
    cp -a "$seed"/. "$PUB_CACHE"/ 2>/dev/null || true
    touch "$PUB_CACHE/.collectemoi-seeded" 2>/dev/null || true
  fi
  # Keep repo → data symlink for collectemoi-app PUBLISH path.
  if [ -d "$COLLECTEMOI_REPO" ] && [ ! -e "$COLLECTEMOI_REPO/data" ]; then
    ln -sfn "$DATA_DIR" "$COLLECTEMOI_REPO/data" 2>/dev/null || true
  fi
  if [ "$(id -u)" -eq 0 ] && getent passwd "$RUN_USER" >/dev/null 2>&1; then
    chown -R "$RUN_USER:$RUN_USER" "$DATA_DIR" || true
  fi
}

# Drop privileges when the image is started as root.
run_as() {
  if [ "$(id -u)" -eq 0 ] && getent passwd "$RUN_USER" >/dev/null 2>&1; then
    exec runuser -u "$RUN_USER" -- "$@"
  fi
  exec "$@"
}

ensure_data_dir

role="${1:-api}"
if [ "$#" -gt 0 ]; then
  shift
fi

case "$role" in
  api)
    run_as /usr/local/bin/collectemoi-api \
      --data-dir "$DATA_DIR" \
      --bind "$BIND" \
      --public-url "$PUBLIC_URL" \
      --http-bind "$HTTP_BIND" \
      --lab-seal "$LAB_SEAL" \
      "$@"
    ;;
  worker)
    run_as /usr/local/bin/collectemoi-worker \
      --data-dir "$DATA_DIR" \
      --poll-secs "$WORKER_POLL" \
      "$@"
    ;;
  all)
    # Single-container convenience: worker in background, API in foreground.
    if [ "$(id -u)" -eq 0 ] && getent passwd "$RUN_USER" >/dev/null 2>&1; then
      runuser -u "$RUN_USER" -- /usr/local/bin/collectemoi-worker \
        --data-dir "$DATA_DIR" \
        --poll-secs "$WORKER_POLL" &
      worker_pid=$!
      trap 'kill "$worker_pid" 2>/dev/null || true' EXIT INT TERM
      runuser -u "$RUN_USER" -- /usr/local/bin/collectemoi-api \
        --data-dir "$DATA_DIR" \
        --bind "$BIND" \
        --public-url "$PUBLIC_URL" \
        --http-bind "$HTTP_BIND" \
        --lab-seal "$LAB_SEAL" \
        "$@"
    else
      /usr/local/bin/collectemoi-worker \
        --data-dir "$DATA_DIR" \
        --poll-secs "$WORKER_POLL" &
      worker_pid=$!
      trap 'kill "$worker_pid" 2>/dev/null || true' EXIT INT TERM
      /usr/local/bin/collectemoi-api \
        --data-dir "$DATA_DIR" \
        --bind "$BIND" \
        --public-url "$PUBLIC_URL" \
        --http-bind "$HTTP_BIND" \
        --lab-seal "$LAB_SEAL" \
        "$@"
    fi
    ;;
  -h|--help|help)
    cat <<EOF
CollecteMoi Docker entrypoint

Roles:
  api      Run the HTTPS API (default)
  worker   Run the decrypt worker
  all      Run worker + API in one container

Environment:
  COLLECTEMOI_DATA_DIR           (default /data)
  COLLECTEMOI_BIND               (default 0.0.0.0:8080)
  COLLECTEMOI_PUBLIC_URL         (default https://127.0.0.1:8080)
  COLLECTEMOI_HTTP_BIND          (default 0.0.0.0:8081, plain HTTP companion)
  COLLECTEMOI_LAB_SEAL           (default true)
  COLLECTEMOI_WORKER_POLL_SECS   (default 2)
  COLLECTEMOI_OTP_WEBHOOK_URL
  COLLECTEMOI_OTP_WEBHOOK_TOKEN

Examples:
  ./scripts/collectemoi-docker up
  ./scripts/collectemoi-docker update --version 1.4.1   # keeps /data volume
  docker run --rm -p 8080:8080 -p 8081:8081 -v collectemoi-data:/data collectemoi:local all

Data: mount a named volume on /data (default collectemoi-data). Image updates
must not remove that volume — use collectemoi-docker update|load, never wipe-data
unless you intend a clean slate.
EOF
    ;;
  *)
    echo "unknown role: $role (expected api|worker|all)" >&2
    exit 2
    ;;
esac
