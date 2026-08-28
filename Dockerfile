# syntax=docker/dockerfile:1

# ---- Stage 1: build the frontend bundle -------------------------------------
FROM node:26-slim@sha256:4ebb5ace66f15a24c14c492e01a8beeed4fddf970a856109f5126e703e5fe503 AS frontend
# pnpm via npm, not corepack: Node 26 ships without corepack (it was unbundled
# upstream), so `corepack enable` is a command-not-found in this image. The
# pinned major is what pnpm-lock.yaml was written by.
RUN npm install -g pnpm@9
WORKDIR /src
COPY pnpm-workspace.yaml package.json pnpm-lock.yaml ./
# Every workspace member's manifest must land before install: @srelens/desktop
# depends on @srelens/core as workspace:*, and pnpm cannot link a package whose
# package.json is not in the image.
COPY apps/desktop/package.json apps/desktop/package.json
COPY packages/core/package.json packages/core/package.json
COPY packages/ui-kit/package.json packages/ui-kit/package.json
COPY packages/ui-next/package.json packages/ui-next/package.json
RUN pnpm install --frozen-lockfile
# Sources after install, so a source-only change does not re-run install.
COPY packages/core packages/core
COPY packages/ui-kit packages/ui-kit
COPY packages/ui-next packages/ui-next
COPY apps/desktop apps/desktop
RUN pnpm --filter @srelens/desktop build

# ---- Stage 2: build the headless server binary ------------------------------
FROM rust:1-slim-bookworm@sha256:94e9efa4033213dbb70d4f665527e7ece3944ddb7ba1dd2e43f6fd6e2490af58 AS backend
WORKDIR /src
# Only C toolchain + perl are needed (no GTK/webkit — this binary isn't Tauri).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config perl make && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY apps/desktop/src-tauri apps/desktop/src-tauri
# rust-embed reads apps/desktop/dist at compile time; copy the built bundle in.
COPY --from=frontend /src/apps/desktop/dist apps/desktop/dist
RUN cargo build --release -p srelens-server --bin srelens-server
RUN strip target/release/srelens-server

# ---- Stage 3: slim runtime --------------------------------------------------
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime
ARG KUBECTL_VERSION=v1.36.3
# Helm is pinned to the 3.x line on purpose: Helm 4 has breaking CLI/behavior
# changes the helm capabilities aren't validated against yet.
ARG HELM_VERSION=v3.21.3
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates bash curl jq tar \
    && rm -rf /var/lib/apt/lists/* \
    # kubectl (downloaded, then verified against upstream-published sha256 before chmod)
    && curl -fsSL "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${TARGETARCH}/kubectl" \
       -o /usr/local/bin/kubectl \
    && echo "$(curl -fsSL "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${TARGETARCH}/kubectl.sha256")  /usr/local/bin/kubectl" | sha256sum -c - \
    && chmod 0755 /usr/local/bin/kubectl \
    # helm (downloaded to a file, verified against upstream-published sha256sum, then extracted)
    && curl -fsSL "https://get.helm.sh/helm-${HELM_VERSION}-linux-${TARGETARCH}.tar.gz" -o /tmp/helm.tgz \
    && echo "$(curl -fsSL "https://get.helm.sh/helm-${HELM_VERSION}-linux-${TARGETARCH}.tar.gz.sha256sum" | awk '{print $1}')  /tmp/helm.tgz" | sha256sum -c - \
    && tar -xz -C /tmp -f /tmp/helm.tgz && mv "/tmp/linux-${TARGETARCH}/helm" /usr/local/bin/helm \
       && chmod 0755 /usr/local/bin/helm && rm -rf /tmp/helm.tgz "/tmp/linux-${TARGETARCH}" \
    # non-root user + owned data dir (GID pinned to match k8s runAsGroup/fsGroup)
    && groupadd -g 10001 srelens \
    && useradd --system --uid 10001 --gid 10001 --create-home --home-dir /home/srelens srelens \
    && mkdir -p /data && chown srelens:srelens /data
COPY --from=backend /src/target/release/srelens-server /usr/local/bin/srelens-server
USER srelens
ENV SRELENS_DATA=/data
# Belt-and-suspenders glibc arena cap (mostly moot with jemalloc as the
# binary's global allocator, but a safe default if that ever changes).
ENV MALLOC_ARENA_MAX=2
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["srelens-server", "serve", "0.0.0.0:8080"]
