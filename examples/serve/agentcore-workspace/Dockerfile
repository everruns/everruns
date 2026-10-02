# AgentCore Runtime runs linux/arm64 images. Build from the repository root:
#   docker buildx build --platform linux/arm64 \
#     -f examples/serve/agentcore-workspace/Dockerfile -t agentcore-workspace .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p serve-example-agentcore-workspace

# Not distroless: the agent's shell needs one, plus the tools it is expected
# to use. Add what your agent needs (python3, node, a compiler) here.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends bash ca-certificates curl git jq \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/agentcore-workspace /usr/local/bin/agentcore-workspace
# The microVM is the session's sandbox, so the agent runs as its only user.
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/agentcore-workspace"]
