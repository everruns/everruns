# syntax=docker/dockerfile:1.7
#
# Isolated sandbox images: what an Everruns "Isolated sandbox" pod runs.
#
#   ghcr.io/everruns/sandbox-dev       --target dev
#   ghcr.io/everruns/sandbox-desktop   --target desktop
#
# Decision: both derive from Microsoft's Dev Container base image, so a team
# that already has a devcontainer.json gets the same userland, and the
# sandbox guest agent is the only Everruns piece. Teams with their own image
# add the guest with the Dev Container Feature in docker/sandbox/feature/.
#
# Decision: the guest binary is static (musl) and copied in, never built
# here: docker-publish.yml compiles it natively per architecture with the
# other release binaries. Context is this directory, with the binary at
# bin/<TARGETARCH>/everruns-sandbox-guest. Locally:
#
#   cargo build --release --locked -p everruns-integrations --features sandbox-guest \
#     --bin everruns-sandbox-guest --target x86_64-unknown-linux-musl
#   mkdir -p docker/sandbox/bin/amd64
#   cp target/x86_64-unknown-linux-musl/release/everruns-sandbox-guest docker/sandbox/bin/amd64/
#   docker buildx build docker/sandbox --target dev -t sandbox-dev:local
#
# The pod runs the guest as its command (kubernetes_sandbox::spec), so the
# image's own entrypoint and user do not matter to Everruns. Commands run as
# root inside the sandbox: with a VM runtime class that root is the VM's.

ARG BASE_IMAGE=mcr.microsoft.com/devcontainers/base:2.0.5-ubuntu-24.04

# ============================================================================
# General-purpose development sandbox
# ============================================================================
FROM ${BASE_IMAGE} AS dev

ARG TARGETARCH
ARG NODE_MAJOR=26
ARG GO_VERSION=1.27.4

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    UV_TOOL_BIN_DIR=/usr/local/bin \
    PATH=/usr/local/cargo/bin:/usr/local/go/bin:$PATH

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# Build tools, Python, search and JSON tools.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config libssl-dev cmake \
      python3 python3-dev python3-venv python3-pip pipx \
      ripgrep fd-find jq yq tree file unzip zip xz-utils zstd \
      sqlite3 postgresql-client redis-tools netcat-openbsd dnsutils iputils-ping \
      git-lfs openssh-client rsync less vim nano procps psmisc \
 && ln -sf /usr/bin/fdfind /usr/local/bin/fd \
 && rm -rf /var/lib/apt/lists/*

# Node.js (NodeSource), with pnpm and yarn through corepack (no longer bundled
# with Node 25+, so it comes from npm).
RUN curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && npm install -g corepack \
 && corepack enable \
 && rm -rf /var/lib/apt/lists/*

# Docker CLI and the GitHub CLI from their own apt repositories. The CLI
# talks to a daemon the agent points it at; the sandbox runs none.
RUN install -m 0755 -d /etc/apt/keyrings \
 && curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc \
 && echo "deb [arch=${TARGETARCH} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable" \
      > /etc/apt/sources.list.d/docker.list \
 && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /etc/apt/keyrings/githubcli.gpg \
 && echo "deb [arch=${TARGETARCH} signed-by=/etc/apt/keyrings/githubcli.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends docker-ce-cli docker-buildx-plugin docker-compose-plugin gh \
 && rm -rf /var/lib/apt/lists/*

# uv (Python packages and versions).
RUN curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh

# Go.
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-${TARGETARCH}.tar.gz" | tar -C /usr/local -xz

# Rust (stable, minimal profile plus rustfmt and clippy). World-writable
# homes so any user in the sandbox can add toolchains and crates.
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain stable --component rustfmt --component clippy \
 && chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"

COPY --chmod=0755 bin/${TARGETARCH}/everruns-sandbox-guest /usr/local/bin/everruns-sandbox-guest

RUN mkdir -p /workspace && chown 1000:1000 /workspace
WORKDIR /workspace
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/everruns-sandbox-guest"]

# ============================================================================
# Development sandbox with a desktop
# ============================================================================
# Xvfb, an xfce session, xdotool and screenshot tools: the computer-use
# desktop (x11_desktop) starts the display on first use at the exact size
# the template asks for. Chromium comes from Playwright, because Ubuntu's
# chromium package is a snap that cannot run in a container.
FROM dev AS desktop

ARG PLAYWRIGHT_VERSION=1.65.1

ENV PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      xvfb xauth x11-utils x11-apps xdotool \
      xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-terminal xfconf dbus-x11 \
      imagemagick scrot ffmpeg \
      fonts-dejavu fonts-liberation fonts-noto-color-emoji \
 && rm -rf /var/lib/apt/lists/*

RUN npx --yes "playwright@${PLAYWRIGHT_VERSION}" install --with-deps chromium \
 && chmod -R a+rX "$PLAYWRIGHT_BROWSERS_PATH" \
 && chrome="$(find "$PLAYWRIGHT_BROWSERS_PATH" -type f -name chrome -path '*chrome-linux*' | head -n 1)" \
 && test -n "$chrome" \
 && printf '#!/bin/sh\nexec "%s" --no-sandbox --no-first-run --disable-dev-shm-usage "$@"\n' "$chrome" \
      > /usr/local/bin/chromium \
 && chmod 0755 /usr/local/bin/chromium \
 && ln -sf /usr/local/bin/chromium /usr/local/bin/chromium-browser \
 && rm -rf /var/lib/apt/lists/* /root/.npm

ENV EVERRUNS_GUEST_DESKTOP=1
