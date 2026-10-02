# everruns-serve-agentcore

> Run a [serve](https://crates.io/crates/everruns-serve) app on Amazon Bedrock AgentCore Runtime. Imported as `serve_agentcore`.

[![Crates.io](https://img.shields.io/crates/v/everruns-serve-agentcore.svg)](https://crates.io/crates/everruns-serve-agentcore)
[![Documentation](https://docs.rs/everruns-serve-agentcore/badge.svg)](https://docs.rs/everruns-serve-agentcore)
[![License](https://img.shields.io/crates/l/everruns-serve-agentcore.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

**Experimental.** A hosting target for serve in the [Everruns](https://everruns.com)
ecosystem. Like serve, it is a proof of concept with no compatibility promise.

[AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-service-contract.html)
runs one container per session, each in its own microVM, and talks to it over HTTP
on port 8080. This crate serves that contract around serve's host, so a serve app
deploys to AgentCore unchanged.

## What It Provides

| Route | What it does |
|---|---|
| `GET /ping` | `{"status":"Healthy"}`, or `HealthyBusy` while a turn runs so AgentCore keeps the session alive |
| `POST /invocations` | An AG-UI 1.0 run of the app's agent as server-sent events. The body is AG-UI `RunAgentInput` (the AgentCore AG-UI protocol) or `{"prompt": "..."}` (a plain `InvokeAgentRuntime` call) |
| `/health`, `/v1/...` | serve's own wire API, unchanged |

The AG-UI thread defaults to the AgentCore session id
(`X-Amzn-Bedrock-AgentCore-Runtime-Session-Id`), so one AgentCore session is one
serve session.

## Quick Example

```rust
use serve::prelude::*;

#[agent]
fn assistant() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")
        .instructions("Be brief.")
        .build()
}

#[tokio::main]
async fn main() -> serve::Result {
    serve_agentcore::start(App::builder().discover().build()).await
}
```

With no command (what AgentCore runs) or `agentcore`, the binary serves the AgentCore
contract on `:8080` in serve's `start` mode. `agentcore --dev` uses `dev` mode to try
it locally. Every other command (`dev`, `start`, `eval`, `manifest`, `deploy`) is
serve's own.

## Configuration

| Variable | Meaning |
|---|---|
| `PORT` | Listen port, default `8080` (AgentCore requires 8080) |
| `SERVE_AGENTCORE_AGENT` | The agent `/invocations` runs, default the app's default agent |
| `SERVE_DATA_DIR`, `DATABASE_URL` | Where serve keeps its SQLite session log. Default: the runtime's session storage at `/mnt/workspace/.serve` when mounted, else a temporary directory |
| `SERVE_WORKSPACE` | The agent's workspace. Default: `/mnt/workspace` when mounted |
| `SERVE_GATEWAY_URL`, `SERVE_GATEWAY_KEY` | serve's model gateway. An AgentCore Gateway inference endpoint (`https://<gateway>/inference/v1`) works here, with targets named after providers (`anthropic`, `openai`) so serve's `provider/model` ids route as-is |

AgentCore mounts session storage only when an invocation arrives, so `/ping` answers
without touching storage and the server boots on the first other request.

## Tools and sandbox

`#[tool]`s and MCP connections (including an AgentCore Gateway) work as in any serve
app. With `[sandbox] kind = "microvm"`, each agent gets a real shell and file tools in
the session's workspace: the microVM is the isolation boundary. Under `dev` and `eval`
the same setting falls back to the bashkit virtual shell.

AgentCore requires an `arm64` Linux image. The
[agentcore example](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore)
has a Dockerfile and deploy steps; the
[agentcore-workspace example](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore-workspace)
adds the microVM shell, an approval tool and an `@ag-ui/client` script.

## Limits

- Not yet served: `/ws` (AgentCore's WebSocket transport) and the MCP and A2A protocol ports.
- `ask_user` and approvals park the turn without keeping the session busy, so AgentCore
  may stop the microVM after its idle timeout. serve's pending approvals do not survive
  a restart yet.
- Not yet integrated: AgentCore Memory, Code Interpreter, Browser and Identity.
- Schedules run in-process, which on AgentCore only fires while a session's microVM is up.
  Use EventBridge to call `InvokeAgentRuntime` instead.

## Documentation

- [Serve on AgentCore guide](https://docs.everruns.com/framework/serve-agentcore/)
- [Serve overview](https://docs.everruns.com/framework/serve/)
- [`everruns-serve` API reference](https://docs.rs/everruns-serve)
- [API reference](https://docs.rs/everruns-serve-agentcore)
- [AgentCore AG-UI protocol contract](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-agui-protocol-contract.html)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
