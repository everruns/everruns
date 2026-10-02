# agentcore-workspace (serve, experimental)

A workspace agent for [Amazon Bedrock AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/agents-tools-runtime.html),
built with [serve-agentcore](../../../crates/serve-agentcore). Each AgentCore session
gets its own microVM; this agent gets a real shell and files there, keeps them on
the session's storage, and asks a person before anything leaves the workspace.

- `[sandbox] kind = "microvm"` in `serve.toml` gives the agent `bash` and file
  tools in `/mnt/workspace`. Under `dev`, `eval` and `agentcore --dev` the bashkit
  virtual shell stands in, so local runs touch nothing on your machine.
- `share_report` is a `#[tool(needs_approval)]`. Over AG-UI the run ends with a
  `tool_approval` interrupt, and the client's next run answers it.
- Session storage keeps both the workspace and serve's session log, so a session
  id that comes back after the idle timeout finds its files and conversation.

See the [Serve on AgentCore guide](https://docs.everruns.com/framework/serve-agentcore/)
for the full contract.

## Run it locally

```sh
cargo run -p serve-example-agentcore-workspace -- eval            # offline eval
cargo run -p serve-example-agentcore-workspace -- agentcore --dev # :8080, offline simulator
```

Drive it with `@ag-ui/client`, approving the share (or `npm start -- reject`):

```sh
cd examples/serve/agentcore-workspace/client
npm install && npm start
# session …
# [RUN_STARTED]
# [RUN_FINISHED]
# interrupt call_llmsim_1_0: tool_approval -> allow
# [RUN_STARTED]
# Added a todo to notes/todo.md and shared it. (Offline demo reply.)
# [TEXT_MESSAGE_END]
# [RUN_FINISHED]
```

## Deploy

The image is Debian slim rather than distroless, because the agent needs a shell
and the tools it is expected to use. Add more to the `Dockerfile` as needed.

```sh
docker buildx build --platform linux/arm64 \
  -f examples/serve/agentcore-workspace/Dockerfile -t "$ECR_REPO:latest" --push .

aws bedrock-agentcore-control create-agent-runtime \
  --agent-runtime-name serve_agentcore_workspace \
  --agent-runtime-artifact "containerConfiguration={containerUri=$ECR_REPO:latest}" \
  --role-arn "$EXECUTION_ROLE_ARN" \
  --network-configuration networkMode=PUBLIC \
  --protocol-configuration serverProtocol=AGUI \
  --filesystem-configurations '[{"sessionStorage":{"mountPath":"/mnt/workspace"}}]' \
  --environment-variables "SERVE_GATEWAY_URL=$GATEWAY_URL/inference/v1,SERVE_GATEWAY_KEY=$GATEWAY_TOKEN"
```

Then point the client at the runtime. Reuse `AGENTCORE_SESSION` to come back to
the same workspace:

```sh
AGENTCORE_URL="https://bedrock-agentcore.$REGION.amazonaws.com/runtimes/$(node -p 'encodeURIComponent(process.argv[1])' "$AGENT_ARN")/invocations?qualifier=DEFAULT" \
AGENTCORE_TOKEN="$BEARER_TOKEN" npm start
```

## Tools from an AgentCore Gateway

A Gateway is an MCP server. Add a connection and every agent gets its tools; `start`
mode refuses to boot without the token, so a misconfigured runtime fails loudly:

```rust
#[connection]
fn gateway() -> McpServer {
    McpServer::http("https://<gateway-id>.gateway.bedrock-agentcore.<region>.amazonaws.com/mcp")
        .auth(Secret::named("GATEWAY_TOKEN"))
}
```

| File | Is |
|---|---|
| `serve.toml` | `[sandbox] kind = "microvm"` |
| `src/agent.rs` | `#[agent] fn assistant()`, with an offline script |
| `src/tools.rs` | `#[tool(needs_approval)] async fn share_report(...)` |
| `evals/workspace.rs` | uses the shell, then shares only after approval |
| `client/run.mjs` | an `@ag-ui/client` run with the approval round trip |
| `Dockerfile` | the `arm64` image, with a shell |
