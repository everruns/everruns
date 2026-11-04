# syntax=docker/dockerfile:1.7
FROM rust:1.96-slim-trixie AS build
WORKDIR /src
COPY . .
# The optional secret extends Cargo's normal trust for managed build proxies.
RUN --mount=type=secret,id=proxy_ca \
    if [ -s /run/secrets/proxy_ca ]; then export CARGO_HTTP_CAINFO=/run/secrets/proxy_ca; fi; \
    cargo build --locked --release -p everruns-trace-store

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/everruns-trace-store /app/everruns-trace-store
USER nonroot:nonroot
ENTRYPOINT ["/app/everruns-trace-store"]
CMD ["run"]
