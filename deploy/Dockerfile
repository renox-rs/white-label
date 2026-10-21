# syntax=docker/dockerfile:1
#
# Build the `host` binary (the one binary every client runs), then run it from
# a small image. Build from the repository root:
#
#   docker build -f deploy/Dockerfile -t white-label-host .
#
# Views, translations and public files are compiled into the binary
# (`renox::embedded!()`, used when APP_DEBUG is off) and host/public/app.css is
# committed, so there is no Tailwind step and nothing to copy next to the
# binary. The client's client.toml is NOT in the image: mount it and point
# CLIENT_CONFIG at it (deploy/README.md).

# Rust 1.94 is the workspace's rust-version (Renox 1.2).
FROM rust:1.94-bookworm AS build
WORKDIR /src
COPY . .
# Cache mounts keep the registry and target dir between builds on one machine.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 \
    cargo build --release -p host --locked \
 && mkdir /out && cp target/release/host /out/host

# `docker build --target binary --output type=local,dest=dist .` writes
# dist/host: the same binary the image runs (Debian 12 glibc), for the VPS
# install without Docker.
FROM scratch AS binary
COPY --from=build /out/host /host

FROM debian:bookworm-slim
# curl is for the HEALTHCHECK only.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /app --uid 10001 app \
 && mkdir -p /app/storage /etc/white-label \
 && chown app /app/storage
WORKDIR /app
COPY --from=build /out/host /app/host
ENV APP_ENV=production \
    APP_DEBUG=false \
    APP_HOST=0.0.0.0 \
    APP_PORT=3000 \
    APP_TIMEZONE=Asia/Dubai \
    STORAGE_PATH=/app/storage \
    DATABASE_URL=sqlite:///app/storage/app.db?mode=rwc \
    CLIENT_CONFIG=/etc/white-label/client.toml
USER app
VOLUME /app/storage
EXPOSE 3000
# GET /health is 200 while the database answers (503 otherwise). It answers
# for any Host, so TRUSTED_HOSTS does not block it.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${APP_PORT}/health" || exit 1
# Pending migrations run on every start. Required at run time: APP_KEY, APP_URL
# and a mounted client.toml (see deploy/env.production.example).
CMD ["sh", "-c", "/app/host migrate && exec /app/host"]
