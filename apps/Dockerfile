# syntax=docker/dockerfile:1

# Default arch; podman build sets --platform=linux/{amd64,arm64} and
# --build-arg TARGETARCH=... in lock-step. TARGETARCH selects which
# api-builder-<arch> stage the `api-builder` alias resolves to, so the arm64
# path never instantiates the Rust toolchain stage at all.
ARG TARGETARCH=amd64
# Set to "prebuilt" to COPY $PREBUILT_API on amd64 too instead of compiling
# (the release workflow builds on the runner, where its cargo cache lives).
# One ARG per FROM: buildah skips the stage when two ARGs are concatenated.
ARG AMD64_API=compile

# ---- ui builder ----
# Pinned to the build host's native platform: the output is plain JS/CSS,
# arch-independent, so it must never run under qemu emulation for the
# arm64 leg of a multiarch build (rolldown's native chunk-rendering step
# is unusably slow emulated — minutes instead of seconds).
FROM --platform=$BUILDPLATFORM docker.io/oven/bun:1-alpine AS ui-builder
WORKDIR /ui
COPY ui/package.json ui/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile
COPY ui/ ./
ARG VITE_API_URL=""
# Public Turnstile sitekey, not a secret — safe to default here so prebuilt
# images bake it in without a build-arg. TURNSTILE_SECRET (server-side) is
# runtime env, never a build arg.
ARG VITE_TURNSTILE_SITE_KEY="0x4AAAAAAEJF4ZVxaxz5WzUH"
ENV VITE_API_URL=${VITE_API_URL}
ENV VITE_TURNSTILE_SITE_KEY=${VITE_TURNSTILE_SITE_KEY}
RUN bun run build
RUN test -f /ui/build/404.html
# Remove Monaco language workers we don't use (plaintext + json only)
RUN rm -f /ui/build/_app/immutable/workers/ts.worker-*.js* \
          /ui/build/_app/immutable/workers/css.worker-*.js* \
          /ui/build/_app/immutable/workers/html.worker-*.js*

# ---- api builder: amd64 (compile natively) ----
FROM docker.io/rust:1-slim-trixie AS api-builder-compile
# Cargo features baked into the release binary. `saml` needs xmlsec1 (bindgen
# reads its headers, hence libclang); pass an empty value to build without it.
ARG CARGO_FEATURES=saml
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config build-essential perl \
    libxmlsec1-dev libclang-dev sccache \
    && rm -rf /var/lib/apt/lists/*
COPY api/Cargo.toml api/Cargo.lock ./
# sccache wraps rustc and stores compiled rlibs in a persistent cache mount
# (target=/sccache). Unlike the image-layer cache, it survives `--no-cache`
# builds and the post-push image pruning in `release-dockerhub`, so
# dependencies compile once, not once per release.
RUN --mount=type=cache,target=/sccache,id=sccache \
    SCCACHE_DIR=/sccache SCCACHE_CACHE_SIZE=20G RUSTC_WRAPPER=sccache \
    mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release --features "${CARGO_FEATURES}" \
    && rm -rf src
COPY api/src/ ./src/
COPY api/migrations/ ./migrations/
# railpack.rs embeds these via include_str!() at compile time — the path is
# relative to /app/src/deployment/git/railpack.rs in this stage, so four
# levels up resolves to container root. Must land at these exact paths.
COPY Dockerfile.railpack-builder /Dockerfile.railpack-builder
COPY scripts/railpack-build.sh /scripts/railpack-build.sh
COPY scripts/railpack-prepare.sh /scripts/railpack-prepare.sh
RUN --mount=type=cache,target=/sccache,id=sccache \
    SCCACHE_DIR=/sccache SCCACHE_CACHE_SIZE=20G RUSTC_WRAPPER=sccache \
    touch src/main.rs && cargo build --release --features "${CARGO_FEATURES}" \
    && cp /app/target/release/litepod-api /app/api-binary

# ---- api builder: prebuilt (take the host's binary, e.g. `cross` output) ----
# No Rust toolchain and no cargo here on purpose: this stage runs under qemu
# on an amd64 host, so anything compiled here would be emulated. The binary
# was already produced natively-cross by `cross build --target
# aarch64-unknown-linux-gnu` (the `release-dockerhub` Justfile recipe drives
# that). $PREBUILT_API is relative to the build context root; the
# .containerignore has a matching un-ignore rule for it.
FROM docker.io/library/debian:trixie-slim AS api-builder-prebuilt
ARG PREBUILT_API
WORKDIR /app
COPY ${PREBUILT_API} /app/api-binary
COPY api/migrations/ ./migrations/

# arm64 is always prebuilt; amd64 only when AMD64_API=prebuilt.
FROM api-builder-${AMD64_API} AS api-builder-amd64
WORKDIR /app
FROM api-builder-prebuilt AS api-builder-arm64
WORKDIR /app

# ---- api builder: arch selector ----
# Resolves to api-builder-amd64 or api-builder-arm64. An unsupported
# TARGETARCH fails here with "no such stage", before anything is built.
FROM api-builder-${TARGETARCH} AS api-builder

# ---- runtime ----
# Trixie, not Bookworm: Bookworm ships Podman 4.3.1, which has no `compose`
# subcommand (added in 4.7). The API shells out to `podman ... compose ...`.
FROM docker.io/library/debian:trixie-slim AS runtime
ARG TARGETARCH
# OpenSSH client is used only for litepod's key-only remote server management and relays.
# sqlite3 is for local inspection of the embedded database file.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl openssh-client podman podman-compose sqlite3 libxmlsec1t64 libxmlsec1t64-openssl git git-lfs \
    && rm -rf /var/lib/apt/lists/*

# Debian/Ubuntu's stock Podman ships without `unqualified-search-registries`
# set (see api/src/servers/setup.rs), so a short image name like
# `postgres:17` fails to resolve. The API's own pre-pull step
# (api/src/deployment/compose/pull.rs) always runs inside this container
# against its local Podman, so it needs this drop-in too — same file and
# contents as `configure_default_registry()` in scripts/install.sh /
# install-cloud.sh and `install_default_registry()` in servers/setup.rs.
RUN mkdir -p /etc/containers/registries.conf.d \
    && printf 'unqualified-search-registries = ["docker.io"]\n' \
        > /etc/containers/registries.conf.d/99-litepod-default-registry.conf

WORKDIR /app
COPY --from=api-builder /app/api-binary /app/api
# Operator CLI (`podman exec litepod-api litepod-api <command>`, see docs).
RUN ln -s /app/api /usr/local/bin/litepod-api
COPY --from=api-builder /app/migrations /app/migrations
COPY --from=ui-builder /ui/build /app/ui

ENV UI_DIST_PATH=/app/ui
ENV PORT=6001

# Build-time version info (set by podx/scripts/build-and-push.sh or just release-dockerhub)
ARG LITEPOD_VERSION=dev
ARG LITEPOD_GIT_SHA=unknown
ENV LITEPOD_VERSION=${LITEPOD_VERSION}
ENV LITEPOD_GIT_SHA=${LITEPOD_GIT_SHA}

LABEL org.opencontainers.image.title="litepod"
LABEL org.opencontainers.image.description="litepod — pod management UI + API (single image)"
LABEL org.opencontainers.image.source="https://github.com/litepod/litepod"
LABEL org.opencontainers.image.version=${LITEPOD_VERSION}
LABEL org.opencontainers.image.revision=${LITEPOD_GIT_SHA}
LABEL org.opencontainers.image.licenses="MIT"

EXPOSE 6001

ENTRYPOINT ["/app/api"]
