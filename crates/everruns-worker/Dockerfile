# Build stage
FROM rust:1.83-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY rust-toolchain.toml ./
COPY crates/everruns-api crates/everruns-api
COPY crates/everruns-contracts crates/everruns-contracts
COPY crates/everruns-storage crates/everruns-storage
COPY crates/everruns-worker crates/everruns-worker

# Build release binary
RUN cargo build --release -p everruns-worker

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/everruns-worker /app/everruns-worker

# Set environment variables
ENV RUST_LOG=info

# Run the binary
CMD ["/app/everruns-worker"]
