# syntax=docker/dockerfile:1
FROM rust:1.96-bookworm AS build
RUN --mount=type=secret,id=system_ca \
    apt_options=""; if [ -f /run/secrets/system_ca ]; then apt_options="-o Acquire::https::CaInfo=/run/secrets/system_ca"; fi; \
    apt-get $apt_options update && apt-get $apt_options install -y --no-install-recommends clang cmake libclang-dev libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=secret,id=system_ca \
    if [ -f /run/secrets/system_ca ]; then export CARGO_HTTP_CAINFO=/run/secrets/system_ca; fi; \
    cargo build --locked --release -p everruns-custom-data-agent

FROM debian:bookworm-slim
RUN --mount=type=secret,id=system_ca \
    apt_options=""; if [ -f /run/secrets/system_ca ]; then apt_options="-o Acquire::https::CaInfo=/run/secrets/system_ca"; fi; \
    apt-get $apt_options update && apt-get $apt_options install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/everruns-custom-data-agent /usr/local/bin/custom-data-agent
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/custom-data-agent"]
