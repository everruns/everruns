# syntax=docker/dockerfile:1.4

# ============================================================================
# Stage 1: Chef - Prepare cargo-chef for dependency caching
# ============================================================================
FROM rust:1.92-slim AS chef

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    curl \
    protobuf-compiler \
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

RUN cargo build --release -p everruns-api

# ============================================================================
# Stage 4: Runtime - Distroless production image (minimal size and security)
# ============================================================================
FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/everruns-api /app/everruns-api

EXPOSE 9000

ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=9000

CMD ["/app/everruns-api"]
