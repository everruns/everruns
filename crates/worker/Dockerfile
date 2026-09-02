# syntax=docker/dockerfile:1.4

# ============================================================================
# Stage 1: Chef - Prepare cargo-chef for dependency caching
# ============================================================================
FROM rust:1.98.0-slim AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt

RUN cargo install cargo-chef --locked

WORKDIR /app

# ============================================================================
# Stage 2: Planner - Analyze dependencies
# ============================================================================
FROM chef AS planner

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY integrations integrations
COPY docs docs

RUN cargo chef prepare --recipe-path recipe.json

# ============================================================================
# Stage 3: Builder - Build dependencies (cached) then application
# ============================================================================
FROM chef AS builder

# Copy recipe and build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy source and build application
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY integrations integrations
COPY docs docs

RUN cargo build --release -p everruns-worker

# ============================================================================
# Stage 4: Runtime - Distroless production image (minimal size and security)
# ============================================================================
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/everruns-worker /app/everruns-worker

ENV RUST_LOG=info

CMD ["/app/everruns-worker"]
