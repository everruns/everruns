# The serve container a celld cell supervises. Build from the repository root:
#   docker build -f examples/serve/celld/Dockerfile -t serve-celld-example .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p serve-example-celld

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/celld-agent /celld-agent
# The cell's `defaultPort`. State lives in /tmp/serve-celld, the same path in
# every container of this image, which a restore relies on.
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["/celld-agent"]
