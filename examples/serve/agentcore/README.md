# agentcore (serve, experimental)

A [serve](../../../crates/serve) app packaged for
[Amazon Bedrock AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-agui-protocol-contract.html)
with [serve-agentcore](../../../crates/serve-agentcore). The app is ordinary serve
code; only `main` changes, from `serve::start` to `serve_agentcore::start`.
The full guide is [Serve on AgentCore](https://docs.everruns.com/framework/serve-agentcore/);
[agentcore-workspace](../agentcore-workspace) adds a shell in the microVM and approvals.

## Try the contract locally

```sh
cargo run -p serve-example-agentcore -- agentcore --dev   # :8080, offline simulator
```

```sh
curl -s localhost:8080/ping
# {"status":"Healthy"}

curl -N localhost:8080/invocations \
  -H 'content-type: application/json' \
  -H 'x-amzn-bedrock-agentcore-runtime-session-id: local-session-0000000000000000000000001' \
  -d '{"prompt":"Where is order A-1001?"}'
# data: {"type":"RUN_STARTED","threadId":"local-session-…",…}
# …
# data: {"type":"RUN_FINISHED",…}
```

`/invocations` also takes a full AG-UI `RunAgentInput`, which is what AgentCore's AG-UI
protocol forwards from CopilotKit or `@ag-ui/client`.

`cargo run -p serve-example-agentcore -- dev` and `-- eval` are serve's usual commands.

## Deploy

1. Build and push an `arm64` image (from the repository root):

   ```sh
   docker buildx build --platform linux/arm64 \
     -f examples/serve/agentcore/Dockerfile -t "$ECR_REPO:latest" --push .
   ```

2. Create the runtime with the AG-UI protocol (or `HTTP` for plain
   `{"prompt": ...}` callers) and [session storage](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-persistent-filesystems.html),
   so a session's conversation survives the microVM stopping after its idle
   timeout. `SERVE_GATEWAY_URL` routes the model; an AgentCore Gateway inference
   endpoint works, with targets named after providers (`anthropic`, `openai`) so
   `anthropic/claude-sonnet-5` routes as-is:

   ```sh
   aws bedrock-agentcore-control create-agent-runtime \
     --agent-runtime-name serve_agentcore_example \
     --agent-runtime-artifact "containerConfiguration={containerUri=$ECR_REPO:latest}" \
     --role-arn "$EXECUTION_ROLE_ARN" \
     --network-configuration networkMode=PUBLIC \
     --protocol-configuration serverProtocol=AGUI \
     --filesystem-configurations '[{"sessionStorage":{"mountPath":"/mnt/workspace"}}]' \
     --environment-variables "SERVE_GATEWAY_URL=$GATEWAY_URL/inference/v1,SERVE_GATEWAY_KEY=$GATEWAY_TOKEN"
   ```

   serve finds the mount on the first invocation and keeps its session log in
   `/mnt/workspace/.serve`.

3. Invoke it with any AG-UI client, or:

   ```sh
   aws bedrock-agentcore invoke-agent-runtime \
     --agent-runtime-arn "$AGENT_ARN" \
     --runtime-session-id "$(uuidgen)-$(uuidgen)" \
     --payload "$(echo -n '{"prompt":"Where is order A-1001?"}' | base64)" \
     out.txt && cat out.txt
   ```

| File | Is |
|---|---|
| `src/main.rs` | `serve_agentcore::start(...)` instead of `serve::start(...)` |
| `src/agent.rs` | `#[agent] fn assistant()` |
| `src/tools.rs` | `#[tool] async fn order_status(cx, order_id)` |
| `evals/status.rs` | `#[eval] async fn looks_up_the_order(t)` |
| `Dockerfile` | the `arm64` image AgentCore runs |
