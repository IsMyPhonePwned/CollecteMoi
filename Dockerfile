# CollecteMoi full server image — API + worker + Flutter/Android toolchain for branded APK builds.
# Build:  docker build -t collectemoi:local .
#         docker build --build-arg COLLECTEMOI_VERSION=1.4.0 -t collectemoi:1.4.0 .
# Or:     ./scripts/collectemoi-docker --version 1.4.0
# Run:    docker compose up --build
# Or:     docker run --rm -p 8080:8080 -p 8081:8081 -v cm-data:/data collectemoi:local all
#
# First build downloads Flutter + Android SDK (several GB). Prefer linux/amd64 for
# Google's Android cmdline-tools when building on Apple Silicon:
#   docker build --platform linux/amd64 -t collectemoi:local .

# Release label baked into the API/worker binaries and shown in the admin UI.
# Empty → Cargo.toml workspace version (currently 0.1.0).
ARG COLLECTEMOI_VERSION=

# ── Rust API / worker ────────────────────────────────────────────────────────
FROM rust:1-bookworm AS build-rust

ARG COLLECTEMOI_VERSION=
ENV COLLECTEMOI_VERSION=${COLLECTEMOI_VERSION}

RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev perl pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --locked --release -p collectemoi-api -p collectemoi-worker \
    && strip target/release/collectemoi-api target/release/collectemoi-worker

# ── Flutter + Android SDK ────────────────────────────────────────────────────
FROM eclipse-temurin:17-jdk-jammy AS build-flutter

ARG FLUTTER_VERSION=stable
ARG ANDROID_CMDLINE_TOOLS=13114758
ARG ANDROID_PLATFORM=android-36
ARG ANDROID_PLATFORM_35=android-35
ARG ANDROID_BUILD_TOOLS=36.0.0
# Flutter 3.50 ships with NDK r28c by default (flutter.ndkVersion).
ARG ANDROID_NDK=28.2.13676358

ENV DEBIAN_FRONTEND=noninteractive \
    FLUTTER_HOME=/opt/flutter \
    FLUTTER_ROOT=/opt/flutter \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    PUB_CACHE=/opt/pub-cache \
    PATH=/opt/flutter/bin:/opt/flutter/bin/cache/dart-sdk/bin:/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git unzip xz-utils zip \
        libglu1-mesa python3 python3-pil \
        fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

# Android command-line tools (x86_64; use --platform linux/amd64 on arm hosts).
RUN mkdir -p "${ANDROID_SDK_ROOT}/cmdline-tools" \
    && curl -fsSL "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
         -o /tmp/cmdline-tools.zip \
    && unzip -q /tmp/cmdline-tools.zip -d /tmp \
    && mv /tmp/cmdline-tools "${ANDROID_SDK_ROOT}/cmdline-tools/latest" \
    && rm -f /tmp/cmdline-tools.zip \
    && yes | sdkmanager --licenses >/dev/null \
    && sdkmanager \
         "platform-tools" \
         "platforms;${ANDROID_PLATFORM}" \
         "platforms;${ANDROID_PLATFORM_35}" \
         "build-tools;${ANDROID_BUILD_TOOLS}" \
         "ndk;${ANDROID_NDK}" \
         "cmake;3.22.1"

# Flutter SDK (stable channel / pinned tag via FLUTTER_VERSION).
RUN git clone --depth 1 --branch "${FLUTTER_VERSION}" https://github.com/flutter/flutter.git "${FLUTTER_HOME}" \
    && git config --global --add safe.directory "${FLUTTER_HOME}" \
    && flutter config --no-analytics --enable-android \
    && flutter precache --android \
    && yes | flutter doctor --android-licenses \
    && flutter doctor -v

# Prefetch Dart/Flutter package deps for the CollecteMoi app (Gradle warms on first APK build).
WORKDIR /opt/collectemoi
COPY app ./app
RUN cd app && flutter pub get

# ── Runtime ──────────────────────────────────────────────────────────────────
FROM eclipse-temurin:17-jdk-jammy AS runtime

ARG COLLECTEMOI_VERSION=
LABEL org.opencontainers.image.title="CollecteMoi" \
      org.opencontainers.image.description="CollecteMoi lab API + worker" \
      org.opencontainers.image.version="${COLLECTEMOI_VERSION}"

ENV DEBIAN_FRONTEND=noninteractive \
    FLUTTER_HOME=/opt/flutter \
    FLUTTER_ROOT=/opt/flutter \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    COLLECTEMOI_REPO=/opt/collectemoi \
    COLLECTEMOI_DATA_DIR=/data \
    COLLECTEMOI_VERSION=${COLLECTEMOI_VERSION} \
    PUB_CACHE=/data/.pub-cache \
    GRADLE_USER_HOME=/data/.gradle \
    GRADLE_OPTS=-Dorg.gradle.daemon=false \
    PATH=/opt/flutter/bin:/opt/flutter/bin/cache/dart-sdk/bin:/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash ca-certificates curl git unzip xz-utils zip \
        libglu1-mesa python3 python3-pil imagemagick \
        fonts-dejavu-core \
        # clang/cmake unused at runtime; keep git for Flutter tool internals
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 collectemoi \
    && useradd --system --uid 10001 --gid collectemoi --home-dir /data --shell /usr/sbin/nologin collectemoi \
    && install -d -o collectemoi -g collectemoi -m 0750 /data

COPY --from=build-rust /src/target/release/collectemoi-api /usr/local/bin/collectemoi-api
COPY --from=build-rust /src/target/release/collectemoi-worker /usr/local/bin/collectemoi-worker
COPY --from=build-flutter /opt/flutter /opt/flutter
COPY --from=build-flutter /opt/android-sdk /opt/android-sdk
COPY --from=build-flutter /opt/pub-cache /opt/pub-cache

# App sources + build scripts (admin “Build Android APK” uses these).
WORKDIR /opt/collectemoi
COPY app ./app
COPY scripts/collectemoi-app scripts/overlay-icon-text.swift scripts/overlay-icon-text.py ./scripts/
RUN chmod 0755 /opt/collectemoi/scripts/collectemoi-app \
               /opt/collectemoi/scripts/overlay-icon-text.py \
    && mkdir -p /opt/collectemoi/dist/mobile \
    && ln -sfn /data /opt/collectemoi/data \
    && git config --global --add safe.directory /opt/flutter \
    && cp -a /opt/pub-cache /opt/pub-cache-seed \
    && chown -R collectemoi:collectemoi /opt/collectemoi /opt/flutter /opt/android-sdk /opt/pub-cache /opt/pub-cache-seed

COPY docker/entrypoint.sh /usr/local/bin/collectemoi-entrypoint
RUN chmod 0755 /usr/local/bin/collectemoi-entrypoint \
    && collectemoi-api --version \
    && collectemoi-worker --version \
    && flutter --version \
    && test -x /opt/collectemoi/scripts/collectemoi-app

# Seed pub cache into the data volume on first boot (entrypoint).
ENV COLLECTEMOI_PUB_CACHE_SEED=/opt/pub-cache-seed

WORKDIR /data
VOLUME ["/data"]
EXPOSE 8080 8081

ENV COLLECTEMOI_BIND=0.0.0.0:8080 \
    COLLECTEMOI_PUBLIC_URL=https://127.0.0.1:8080 \
    COLLECTEMOI_HTTP_BIND=0.0.0.0:8081 \
    COLLECTEMOI_LAB_SEAL=true \
    COLLECTEMOI_WORKER_POLL_SECS=2 \
    COLLECTEMOI_USER=collectemoi

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8081/healthz || curl -kfsS https://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/collectemoi-entrypoint"]
CMD ["api"]
