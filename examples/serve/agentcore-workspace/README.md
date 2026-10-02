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

The agent's model is `bedrock/us.anthropic.claude-sonnet-4-6`, called with the
runtime's execution role: no API keys in the image or the runtime. The role
needs `bedrock:InvokeModelWithResponseStream` on the model and its inference
profile, plus the usual ECR pull and CloudWatch Logs permissions.

The image is Debian slim rather than distroless, because the agent needs a shell
and the tools it is expected to use. Add more to the `Dockerfile` as needed.
AgentCore runs `linux/arm64`; without a local arm64 builder, an `ARM_CONTAINER`
CodeBuild project with this `Dockerfile` works.

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
  --environment-variables AWS_REGION=us-east-1
```

## Talk to it

With IAM credentials (`pip install boto3`). The session id is kept in
`.agentcore-session`, so the next call returns to the same microVM, conversation
and files; `--new` starts over:

```sh
python3 client/invoke.py --arn "$AGENT_ARN" "Add 'check the build' to my todo list and share it."
python3 client/invoke.py --arn "$AGENT_ARN" "What is on my todo list?"
```

Over the WebSocket transport (`pip install websockets`), each prompt is one turn
on the same socket; the script signs the upgrade with SigV4 (`--presign` puts
the signature in the URL) and shares the session file with `invoke.py`:

```sh
python3 client/ws.py --arn "$AGENT_ARN" "Add 'try the websocket' to my todo list and share it." "What is on it now?"
```

With a bearer token (JWT inbound auth), the `@ag-ui/client` script works too:

```sh
AGENTCORE_URL="https://bedrock-agentcore.$REGION.amazonaws.com/runtimes/$(node -p 'encodeURIComponent(process.argv[1])' "$AGENT_ARN")/invocations?qualifier=DEFAULT" \
AGENTCORE_TOKEN="$BEARER_TOKEN" npm start
```

Locally, `agentcore` without `--dev` runs the real shell on your machine, in
`SERVE_WORKSPACE`; keep `--dev` unless you mean it.

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
| `client/invoke.py` | a boto3 (IAM) client with the approval round trip |
| `client/ws.py` | the same over `/ws`, with a SigV4-signed upgrade |
| `client/run.mjs` | an `@ag-ui/client` run with the approval round trip |
| `Dockerfile` | the `arm64` image, with a shell |
