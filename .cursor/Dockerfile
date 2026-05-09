FROM mcr.microsoft.com/devcontainers/universal:2-linux

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_INCREMENTAL=0

# NOTE(sync): Keep this file aligned with:
# - scripts/init-cloud-env.sh (tool versions and install expectations)
# - AGENTS.md "Cloud Agent (start here)" section (required runtime commands)
# - .cursor/environment.json (install/start workflow)
# If you bump versions in one place, bump all three.

# System tools required by full-stack local runs.
#
# Valkey: build from the GitHub release tarball so we ship real `valkey-server`
# / `valkey-cli` (licensing) without relying on `download.valkey.io` (often
# blocks automated downloads with HTTP 403) or mixing prebuilt glibc versions.
# Verify the release tarball SHA-256 before extraction to prevent silent
# supply-chain swaps in transit or at the source URL.
#
# The universal devcontainer image ships a Yarn APT source that can break
# `apt-get update` (missing/rotated key). Yarn is provided via other tooling in
# the base image; drop the apt source so package installs remain reliable.
#
# NOTE(sync): Bump VALKEY_VERSION with `examples/docker-compose-full.yaml` /
# production Valkey major line when you intentionally upgrade.
RUN set -eux; \
    rm -f /etc/apt/sources.list.d/yarn.list /etc/apt/sources.list.d/yarn-stable.list; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      build-essential \
      pkg-config \
      libssl-dev \
      postgresql \
      postgresql-client \
      netcat-openbsd \
      jq; \
    VALKEY_VERSION="8.1.7"; \
    VALKEY_SHA256="15c984f3fa7ee5b45a79cf9641e4871bf999acb65654940304e1c0807f58f7bc"; \
    curl -fsSL --retry 3 --retry-delay 2 \
      "https://github.com/valkey-io/valkey/archive/refs/tags/${VALKEY_VERSION}.tar.gz" \
      -o /tmp/valkey.tgz; \
    echo "${VALKEY_SHA256}  /tmp/valkey.tgz" | sha256sum -c - >/dev/null; \
    tar -xzf /tmp/valkey.tgz -C /tmp; \
    make -C "/tmp/valkey-${VALKEY_VERSION}" MALLOC=libc -j"$(nproc)"; \
    make -C "/tmp/valkey-${VALKEY_VERSION}" MALLOC=libc install PREFIX=/usr/local; \
    rm -rf "/tmp/valkey-${VALKEY_VERSION}" /tmp/valkey.tgz; \
    valkey-server --version; \
    valkey-cli --version; \
    apt-get purge -y --auto-remove \
      build-essential \
      pkg-config \
      libssl-dev; \
    rm -rf /var/lib/apt/lists/*

# Preinstall tools that scripts/init-cloud-env.sh expects.
# NOTE(sync): Versions below intentionally mirror scripts/init-cloud-env.sh:
# - GH_VERSION=2.63.2
# - DOP_VERSION=3.75.2
# - CADDY_VERSION=2.9.1
# - NATS_VERSION=2.11.3
RUN set -eux; \
    ARCH="$(dpkg --print-architecture)"; \
    case "$ARCH" in \
      amd64) GH_ARCH="amd64"; DOP_ARCH="amd64"; CADDY_ARCH="amd64"; NATS_ARCH="amd64" ;; \
      arm64) GH_ARCH="arm64"; DOP_ARCH="arm64"; CADDY_ARCH="arm64"; NATS_ARCH="arm64" ;; \
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;; \
    esac; \
    INSTALL_DIR="/usr/local/bin"; \
    curl -fsSL --retry 3 --retry-delay 2 https://just.systems/install.sh | bash -s -- --to "$INSTALL_DIR"; \
    GH_VERSION="2.63.2"; \
    GH_TARBALL="gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz"; \
    curl -fsSL --retry 3 --retry-delay 2 "https://github.com/cli/cli/releases/download/v${GH_VERSION}/${GH_TARBALL}" -o /tmp/"$GH_TARBALL"; \
    tar -xzf /tmp/"$GH_TARBALL" -C /tmp; \
    cp /tmp/"gh_${GH_VERSION}_linux_${GH_ARCH}"/bin/gh "$INSTALL_DIR"/gh; \
    chmod +x "$INSTALL_DIR"/gh; \
    DOP_VERSION="3.75.2"; \
    case "$DOP_ARCH" in \
      amd64) DOP_SHA256="bfc58d21baa3da2e177a74fb7fbbb8529170b560b1363ac5420de56a2786c489" ;; \
      arm64) DOP_SHA256="b2cb9e8312a088f5e87bc3c1a4e0bd3162cf8f8aa8ed73adfc08cc50a32e9f4f" ;; \
      *) echo "Unsupported Doppler architecture: $DOP_ARCH"; exit 1 ;; \
    esac; \
    DOP_TARBALL="doppler_${DOP_VERSION}_linux_${DOP_ARCH}.tar.gz"; \
    curl -fsSL --retry 3 --retry-delay 2 "https://github.com/DopplerHQ/cli/releases/download/${DOP_VERSION}/${DOP_TARBALL}" -o /tmp/"$DOP_TARBALL"; \
    echo "${DOP_SHA256}  /tmp/${DOP_TARBALL}" | sha256sum -c - >/dev/null; \
    tar -xzf /tmp/"$DOP_TARBALL" -C /tmp; \
    cp /tmp/doppler "$INSTALL_DIR"/doppler; \
    chmod +x "$INSTALL_DIR"/doppler; \
    CADDY_VERSION="2.9.1"; \
    CADDY_TARBALL="caddy_${CADDY_VERSION}_linux_${CADDY_ARCH}.tar.gz"; \
    curl -fsSL --retry 3 --retry-delay 2 "https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/${CADDY_TARBALL}" -o /tmp/"$CADDY_TARBALL"; \
    tar -xzf /tmp/"$CADDY_TARBALL" -C /tmp caddy; \
    cp /tmp/caddy "$INSTALL_DIR"/caddy; \
    chmod +x "$INSTALL_DIR"/caddy; \
    NATS_VERSION="2.11.3"; \
    NATS_TARBALL="nats-server-v${NATS_VERSION}-linux-${NATS_ARCH}.tar.gz"; \
    curl -fsSL --retry 3 --retry-delay 2 "https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/${NATS_TARBALL}" -o /tmp/"$NATS_TARBALL"; \
    tar -xzf /tmp/"$NATS_TARBALL" -C /tmp; \
    cp /tmp/"nats-server-v${NATS_VERSION}-linux-${NATS_ARCH}"/nats-server "$INSTALL_DIR"/nats-server; \
    chmod +x "$INSTALL_DIR"/nats-server; \
    rm -rf /tmp/gh_* /tmp/doppler* /tmp/caddy* /tmp/nats-server*; \
    just --version; \
    gh --version; \
    doppler --version; \
    caddy version; \
    nats-server --version

# Keep the default devcontainer user; cloud agents run as non-root.
USER codespace
