FROM rust:1-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev perl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release -p collectemoi-api -p collectemoi-worker

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 collectemoi \
    && useradd --system --uid 10001 --gid collectemoi --home-dir /data collectemoi \
    && install -d -o collectemoi -g collectemoi -m 0750 /data

COPY --from=build /src/target/release/collectemoi-api /usr/local/bin/collectemoi-api
COPY --from=build /src/target/release/collectemoi-worker /usr/local/bin/collectemoi-worker

USER collectemoi:collectemoi
VOLUME ["/data"]
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/collectemoi-api"]
CMD ["--data-dir", "/data", "--bind", "0.0.0.0:8080", "--public-url", "https://127.0.0.1:8080"]
