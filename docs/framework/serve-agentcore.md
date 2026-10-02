---
title: Serve on Amazon Bedrock AgentCore (experimental)
description: Deploy a serve app to Amazon Bedrock AgentCore Runtime with everruns-serve-agentcore, with sessions on session storage, tools in the microVM and models through AgentCore Gateway.
sidebar:
  order: 2
---

> **Experimental.** Like [serve](/framework/serve/), `everruns-serve-agentcore`
> is a proof of concept with no compatibility promise.

[Amazon Bedrock AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/agents-tools-runtime.html)
runs your agent container once per session, each copy in its own microVM, and
talks to it over HTTP on port 8080. `everruns-serve-agentcore` (imported as
`serve_agentcore`) serves that contract around a [serve](/framework/serve/) app,
so the same app runs under `dev` on a laptop and on AgentCore without changes.

## How it maps

| AgentCore | serve |
|---|---|
| Runtime session (`X-Amzn-Bedrock-AgentCore-Runtime-Session-Id`) | One serve session: the AG-UI thread defaults to the session id |
| `POST /invocations` | An AG-UI 1.0 run of the app's agent, streamed as server-sent events |
| `GET /ping` | `Healthy`, or `HealthyBusy` while a turn runs so AgentCore keeps the microVM up |
| Session storage mount (`/mnt/workspace`) | serve's SQLite session log and the agent's workspace |
| The session microVM | The sandbox: `[sandbox] kind = "microvm"` gives the agent a real shell |
| Gateway inference target (`/inference/v1`) | serve's model gateway (`SERVE_GATEWAY_URL`) |
| Gateway MCP target | A `#[connection]` returning `McpServer` |

serve's `/v1` wire API and `/health` stay mounted next to the AgentCore routes.

## Quickstart

Start from any serve app and change `main`:

```rust
use serve::prelude::*;

#[tokio::main]
async fn main() -> serve::Result {
    serve_agentcore::start(App::builder().discover().build()).await
}
```

```toml
[dependencies]
everruns-serve = { version = "0.33", features = ["ag-ui"] }
everruns-serve-agentcore = "0.33"
```

With no command (what AgentCore runs) or `agentcore`, the binary serves the
AgentCore contract in serve's `start` mode. Every other command (`dev`, `start`,
`eval`, `manifest`, `deploy`) is serve's own.

Try the contract locally with the offline simulator:

```sh
cargo run -p serve-example-agentcore -- agentcore --dev

curl -s localhost:8080/ping
# {"status":"Healthy"}

curl -N localhost:8080/invocations \
  -H 'content-type: application/json' \
  -H 'x-amzn-bedrock-agentcore-runtime-session-id: local-session-0000000000000000000000001' \
  -d '{"prompt":"Where is order A-1001?"}'
# data: {"type":"RUN_STARTED",...}
# ...
# data: {"type":"RUN_FINISHED",...}
```

`/invocations` takes either an AG-UI `RunAgentInput` (what AgentCore's AG-UI
protocol forwards from CopilotKit or `@ag-ui/client`) or `{"prompt": "..."}`
(a plain `InvokeAgentRuntime` call). Without a `threadId` and without the
session header, it answers `400` with a problem document.

## Deploy

AgentCore runs `linux/arm64` images on port 8080. The
[agentcore example](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore)
has a distroless Dockerfile; build and push from the repository root:

```sh
docker buildx build --platform linux/arm64 \
  -f examples/serve/agentcore/Dockerfile -t "$ECR_REPO:latest" --push .
```

Create the runtime with the AG-UI protocol and session storage:

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

Use `serverProtocol=HTTP` instead when callers send `{"prompt": ...}`;
`/invocations` accepts both either way.

## Invoke

From the CLI:

```sh
aws bedrock-agentcore invoke-agent-runtime \
  --agent-runtime-arn "$AGENT_ARN" \
  --runtime-session-id "$(uuidgen)-$(uuidgen)" \
  --payload "$(echo -n '{"prompt":"Where is order A-1001?"}' | base64)" \
  out.txt && cat out.txt
```

From an AG-UI client, point `HttpAgent` at the runtime's invocations URL and
send the session header, as the
[workspace example's client](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore-workspace/client)
does. It also shows the approval round trip: the first run ends with an
`interrupt` outcome, and the next run resumes it.

```js
const agent = new HttpAgent({
  url: `https://bedrock-agentcore.${region}.amazonaws.com/runtimes/${encodeURIComponent(agentArn)}/invocations?qualifier=DEFAULT`,
  headers: {
    Authorization: `Bearer ${token}`,
    "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id": session,
  },
  threadId: session,
});
```

Reuse a session id to return to the same microVM, conversation and workspace.

## Persistence

AgentCore mounts session storage only when an invocation arrives, not while the
container initializes. So `/ping` never touches storage, and the server boots
on the first other request. It then picks, in order:

1. `SERVE_DATA_DIR` (or a SQLite `DATABASE_URL`) when set. The workspace is
   `SERVE_WORKSPACE`, else `workspace/` under the data dir.
2. `/mnt/workspace` when it is mounted: the session log goes to
   `/mnt/workspace/.serve`, and the agent's workspace is `/mnt/workspace`.
3. Otherwise a temporary directory, with a warning that nothing survives the
   microVM stopping.

With session storage, a conversation survives AgentCore stopping the microVM
after its idle timeout: the next invocation with the same session id resumes it
from the log. Session storage is per session, kept for 14 days, and reset when
you deploy a new runtime version. For history across sessions and versions,
point `DATABASE_URL` at storage you own, such as an S3 Files or EFS mount
(both need VPC mode).

## Tools

- **In-process tools.** `#[tool]` functions run in the container like anywhere
  else, including `needs_approval` tools: over AG-UI the run ends with a
  `tool_approval` interrupt and the client's next run answers it.
- **A real shell.** With `[sandbox] kind = "microvm"` in `serve.toml`, each
  agent gets `bash` and file tools in the session's workspace, running directly
  in the container. The microVM is the isolation boundary, so there is no
  second sandbox inside it. Under `dev` and `eval` the same setting uses the
  bashkit virtual shell, so local runs stay safe. Install whatever the agent
  should run (git, python, a compiler) in the image, as the
  [workspace example](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore-workspace)
  Dockerfile does.
- **AgentCore Gateway tools.** A Gateway is an MCP server, so connect it like
  any other:

  ```rust
  #[connection]
  fn gateway() -> McpServer {
      McpServer::http("https://<gateway-id>.gateway.bedrock-agentcore.<region>.amazonaws.com/mcp")
          .auth(Secret::named("GATEWAY_TOKEN"))
  }
  ```

  `start` mode, which AgentCore runs, refuses to boot when `GATEWAY_TOKEN` is
  unset, so a misconfigured runtime fails its first invocation instead of
  running without tools.

## Models

serve routes `provider/model` ids through its gateway. Set `SERVE_GATEWAY_URL`
to an AgentCore Gateway inference endpoint (`https://<gateway>/inference/v1`)
with targets named after providers (`anthropic`, `openai`), and
`anthropic/claude-sonnet-5` routes as-is. Provider API keys then stay in the
Gateway's credential providers instead of the runtime's environment.

## Configuration

| Variable | Meaning |
|---|---|
| `PORT` | Listen port, default `8080` (AgentCore requires 8080) |
| `SERVE_AGENTCORE_AGENT` | The agent `/invocations` runs, default the app's default agent |
| `SERVE_DATA_DIR`, `DATABASE_URL` | Where the session log lives. Default: session storage when mounted |
| `SERVE_WORKSPACE` | The agent's workspace. Default: the session storage mount |
| `SERVE_GATEWAY_URL`, `SERVE_GATEWAY_KEY` | serve's model gateway |

## Compared with other AgentCore agents

| | serve-agentcore | Strands / `bedrock-agentcore` SDK |
|---|---|---|
| `/ping`, `/invocations`, busy reporting | Yes | Yes |
| AG-UI protocol | Yes, AG-UI 1.0 including interrupts | Yes |
| `/ws` WebSocket | Not yet | Yes |
| MCP and A2A server protocols | Not yet | Yes |
| Conversation persistence | SQLite on session storage | File or AgentCore Memory session managers |
| Long-term memory (AgentCore Memory) | Not yet | Yes |
| Gateway tools | MCP connection | MCP client |
| Shell and files | Built-in (`sandbox = "microvm"`) | Bring your own tools |
| Code Interpreter, Browser | Not yet | Client libraries |
| Identity (OAuth to third parties) | Secrets from the environment | `@requires_access_token` |
| Offline evals and simulator | Yes (`eval`, `--dev`) | No |

## Limits

- Approvals and `ask_user` questions park a turn without keeping the microVM
  busy, and a parked turn does not survive a restart yet. Answer within the
  idle timeout (15 minutes by default).
- `#[schedule]`s run in-process, so they only fire while a session's microVM
  is up. Use EventBridge to call `InvokeAgentRuntime` on a schedule instead.
- The Bedrock model driver takes static keys only. Route models through an
  AgentCore Gateway, or another gateway, rather than relying on the runtime's
  IAM role.

## Reference

- [`everruns-serve-agentcore` API reference](https://docs.rs/everruns-serve-agentcore)
- [AgentCore AG-UI protocol contract](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-agui-protocol-contract.html)
- [AgentCore session storage](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-persistent-filesystems.html)
