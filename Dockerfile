# syntax=docker/dockerfile:1

# agpeer one-binary container: WebUI dist -> Rust build (`--features webui`)
# -> slim runtime. Exposes the core API + embedded UI on 41000.

# ---- Stage 1: WebUI bundle ----
FROM node:20-alpine AS webui
WORKDIR /src/apps/desktop
COPY apps/desktop/package.json apps/desktop/package-lock.json ./
RUN npm ci
COPY apps/desktop/ ./
RUN npm run build

# ---- Stage 2: Rust build ----
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
# Embed the freshly built Desktop UI.
COPY --from=webui /src/apps/desktop/dist apps/desktop/dist
RUN cargo build --release --features webui --bin agpeer

# ---- Stage 3: slim runtime ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gosu \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/agpeer /usr/local/bin/agpeer
# Provision the layout the shipped example config (deploy/config.example.toml)
# expects plus the headless defaults, all owned by the runtime user so
# AppConfig::ensure_dirs can always create DB/token/download dirs.
RUN mkdir -p /data /downloads \
    /opt/agpeer/downloads /opt/agpeer/soulseek-downloads /opt/agpeer/library \
    && chown -R nobody:nogroup /data /downloads /opt/agpeer

# PUID/PGID entrypoint: starts as root, aligns ownership of writable volumes
# with PUID/PGID (default 65534:65534 = nobody), then drops privileges via
# gosu before exec'ing the binary. The agpeer process itself never runs as
# root.
COPY deploy/docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

ENV AGPEER_HOST=0.0.0.0 \
    AGPEER_DATA_DIR=/data \
    # The runtime user's home is deliberately /nonexistent; point HOME at
    # the writable data volume so tools that use $HOME (rqbit DHT cache)
    # work and persist across restarts.
    HOME=/data

EXPOSE 41000
VOLUME ["/data", "/opt/agpeer/downloads"]

# Probe the unauthenticated embedded UI shell: /api/v1/status requires the
# bearer token, so it can never pass an unauthenticated health probe.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS -o /dev/null http://127.0.0.1:41000/ || exit 1

# The entrypoint performs the privilege drop; do not override with `user:`
# unless you accept running agpeer itself as that user.
USER root
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["serve"]