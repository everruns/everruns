# Devcontainer for Everruns - optimized for Claude Code in Cloud
# Includes: Rust, Node.js, just, PostgreSQL client, Temporal CLI, and development tools

FROM rust:1.83-bookworm

ARG TZ
ENV TZ="$TZ"

ARG CLAUDE_CODE_VERSION=latest

# Install development tools, PostgreSQL client, and firewall utilities
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Essential tools
    less \
    git \
    procps \
    sudo \
    curl \
    wget \
    unzip \
    # Shell enhancements
    fzf \
    zsh \
    man-db \
    # Editors
    nano \
    vim \
    # Build dependencies
    build-essential \
    pkg-config \
    libssl-dev \
    # Protocol buffers (required for Temporal SDK)
    protobuf-compiler \
    # PostgreSQL client
    postgresql-client \
    # Python for yq and other tools
    python3 \
    python3-pip \
    python3-venv \
    # JSON processing
    jq \
    # GitHub CLI
    gnupg2 \
    gh \
    # Firewall tools (for Claude Code security)
    iptables \
    ipset \
    iproute2 \
    dnsutils \
    aggregate \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# Install just command runner
ARG JUST_VERSION=1.36.0
RUN ARCH=$(dpkg --print-architecture) && \
    case ${ARCH} in \
        amd64) JUST_ARCH="x86_64-unknown-linux-musl" ;; \
        arm64) JUST_ARCH="aarch64-unknown-linux-musl" ;; \
        *) echo "Unsupported architecture: ${ARCH}" && exit 1 ;; \
    esac && \
    wget -q "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-${JUST_ARCH}.tar.gz" -O /tmp/just.tar.gz && \
    tar -xzf /tmp/just.tar.gz -C /usr/local/bin just && \
    chmod +x /usr/local/bin/just && \
    rm /tmp/just.tar.gz

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y nodejs && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

# Install yq via pip
RUN pip3 install --break-system-packages yq

# Install git-delta for better diffs
ARG GIT_DELTA_VERSION=0.18.2
RUN ARCH=$(dpkg --print-architecture) && \
    wget -q "https://github.com/dandavison/delta/releases/download/${GIT_DELTA_VERSION}/git-delta_${GIT_DELTA_VERSION}_${ARCH}.deb" -O /tmp/git-delta.deb && \
    dpkg -i /tmp/git-delta.deb && \
    rm /tmp/git-delta.deb

# Install Temporal CLI
RUN ARCH=$(dpkg --print-architecture) && \
    case ${ARCH} in \
        amd64) TEMPORAL_ARCH="amd64" ;; \
        arm64) TEMPORAL_ARCH="arm64" ;; \
        *) echo "Unsupported architecture: ${ARCH}" && exit 1 ;; \
    esac && \
    curl -sSf https://temporal.download/cli.sh | sh -s -- --dir /usr/local/bin

# Create non-root user for development
ARG USERNAME=developer
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN groupadd --gid $USER_GID $USERNAME && \
    useradd --uid $USER_UID --gid $USER_GID -m $USERNAME && \
    echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME && \
    chmod 0440 /etc/sudoers.d/$USERNAME

# Ensure directories exist and set permissions
RUN mkdir -p /usr/local/share/npm-global && \
    chown -R $USERNAME:$USERNAME /usr/local/share/npm-global

# Persist command history
RUN mkdir /commandhistory && \
    touch /commandhistory/.bash_history && \
    touch /commandhistory/.zsh_history && \
    chown -R $USERNAME:$USERNAME /commandhistory

# Set environment variables
ENV DEVCONTAINER=true
ENV NPM_CONFIG_PREFIX=/usr/local/share/npm-global
ENV PATH=$PATH:/usr/local/share/npm-global/bin
ENV SHELL=/bin/zsh
ENV EDITOR=nano
ENV VISUAL=nano

# Create workspace and config directories
RUN mkdir -p /workspace /home/$USERNAME/.claude /home/$USERNAME/.cargo && \
    chown -R $USERNAME:$USERNAME /workspace /home/$USERNAME/.claude /home/$USERNAME/.cargo

WORKDIR /workspace

# Switch to non-root user
USER $USERNAME

# Install Rust components
RUN rustup component add rustfmt clippy && \
    rustup target add wasm32-unknown-unknown 2>/dev/null || true

# Install Rust development tools
RUN cargo install sqlx-cli --no-default-features --features postgres && \
    cargo install cargo-watch && \
    cargo install cargo-deny --locked

# Set up zsh with plugins
ARG ZSH_IN_DOCKER_VERSION=1.2.0
RUN sh -c "$(wget -O- https://github.com/deluan/zsh-in-docker/releases/download/v${ZSH_IN_DOCKER_VERSION}/zsh-in-docker.sh)" -- \
    -p git \
    -p fzf \
    -a "source /usr/share/doc/fzf/examples/key-bindings.zsh" \
    -a "source /usr/share/doc/fzf/examples/completion.zsh" \
    -a "export HISTFILE=/commandhistory/.zsh_history" \
    -a "export HISTSIZE=10000" \
    -a "export SAVEHIST=10000" \
    -x

# Install Claude Code CLI
RUN npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}

# Copy and set up firewall script
COPY init-firewall.sh /usr/local/bin/
USER root
RUN chmod +x /usr/local/bin/init-firewall.sh && \
    echo "$USERNAME ALL=(root) NOPASSWD: /usr/local/bin/init-firewall.sh" > /etc/sudoers.d/$USERNAME-firewall && \
    chmod 0440 /etc/sudoers.d/$USERNAME-firewall
USER $USERNAME
