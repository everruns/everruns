# AgentCore Runtime runs linux/arm64 images. Build from the repository root:
#   docker buildx build --platform linux/arm64 \
#     -f examples/serve/agentcore/Dockerfile -t serve-agentcore .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p serve-example-agentcore

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/agentcore /agentcore
# AgentCore's HTTP and AG-UI contracts: port 8080, /ping and /invocations.
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["/agentcore"]
