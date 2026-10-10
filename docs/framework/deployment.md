---
title: Deploying a Framework App
description: Ship a Framework app as a binary or a container, keep its data directory on persistent storage, pass provider keys through the environment, and know when to use serve instead.
---

A Framework app is your own Rust binary. The `everruns` crate needs no
database, message broker, or Everruns server, so deploying it is deploying that
binary: build it, give it provider credentials, and, if sessions must outlive
the process, give it a directory that persists.

## Choose where state lives

| Engine setup | State | Deploy with |
|---|---|---|
| `Engine::new()` | In memory, lost when the process exits | Any binary or container. Nothing to persist |
| An agent built with `.local(LocalConfig::new(dir))` | Crash-durable events, session catalog, and SQLite task and schedule state under `dir`; agent files under `dir/workspace` unless `.workspace(path)` sets another root | A volume that survives restarts and redeploys |

`LocalConfig` needs the `local` feature. [Sessions](/framework/sessions/#persistence)
covers the full trade-off.

For a deployment that keeps local state:

- **One process per data directory.** The local profile is built for one
  embedded process at a time. Do not point two replicas or two containers at
  the same directory. Scale by giving each instance its own directory and
  routing each session to the instance that owns it. If two processes do
  share one, a session's turns still run in only one of them at a time: each
  turn holds the session's lease in the local database.
- **Trusted paths only.** Choose the data and workspace directories from
  deployment configuration, never from model output or request input.
- **Persistent storage.** Mount the data directory from a volume. New state
  files are created owner-only on Unix; your backups and copies need the same
  protection, because the event log holds conversation content.
- **Resume after a restart.** Rebuild each agent from the same trusted
  configuration, then call `engine.attach(session_id, agent)` and
  `engine.resume(session_id)`. See
  [Resume after a process restart](/framework/sessions/#resume-after-a-process-restart).

## Pass secrets through the environment

Each provider type reads its key with `from_env`: `OpenAI::from_env()` reads
`OPENAI_API_KEY` and `OPENAI_BASE_URL`, `Anthropic::from_env()` reads
`ANTHROPIC_API_KEY`, and so on. The full per-driver table is under
[Credentials](/framework/models-and-providers/#credentials). Set those variables
from your platform's secret store (a container secret, a Kubernetes `Secret`, a
systemd credential), not in the image.

`from_env` returns an error when a required variable is missing, so a
misconfigured deployment fails at startup instead of on the first turn. When
your application already resolves the key itself, pass it with
`OpenAI::new(key)`. On AWS, `Bedrock::default_chain()` uses the instance or
task role, so no key is stored at all.

A minimal service entry point:

```rust
use std::time::Duration;
use everruns::{Agent, Engine, LocalConfig, OpenAI};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
// Deployment configuration, not request input.
let data_dir = std::env::var("APP_DATA_DIR").unwrap_or_else(|_| "/data".into());

let agent = Agent::builder()
    .instructions("Answer support questions.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .local(LocalConfig::new(data_dir))
    .build()?;

let engine = Engine::new();
let session = engine.create(agent);
// ... accept requests and drive sessions ...
# let _ = session;

tokio::signal::ctrl_c().await?;
engine.shutdown(Duration::from_secs(10)).await;
# Ok(())
# }
```

`APP_DATA_DIR` is this example's own setting; the Framework reads no variable
for the data directory. `Engine::shutdown` drains and flushes event listeners
such as the OpenTelemetry and Braintrust exporters, so call it before the
process exits. See [Observability](/framework/observability/).

## Build a container

A multi-stage build keeps the toolchain out of the runtime image. This follows
the Dockerfile in
[AgentCore app](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore):

```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin my-agent

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/my-agent /my-agent
VOLUME /data
ENTRYPOINT ["/my-agent"]
```

The `everruns` crate's HTTP clients use rustls, so the image needs no OpenSSL.
It still needs CA certificates to reach provider APIs; the distroless `cc`
image includes them. The `nonroot` user must be able to write the mounted data
directory.

Run it with the key from a secret and the data directory on a volume:

```bash
docker run --rm \
  -e OPENAI_API_KEY \
  -e APP_DATA_DIR=/data \
  -v my-agent-data:/data \
  my-agent
```

A container without `LocalConfig` needs no volume.

## When to use serve instead

[serve](/framework/serve/) is an experimental application framework on top of
the Framework. Use it instead of a hand-written binary when you want:

- the Everruns `/v1` session API, so the SDKs, CLI, and UI chat view can drive
  your agents, plus AG-UI and A2A endpoints;
- a manifest of agents, tools, secrets, and models that a host can provision
  from;
- a `start` command that requires every declared secret, and that fails a
  model with no real route instead of falling back to the simulator. Hosts
  route models through `SERVE_GATEWAY_URL` and `SERVE_GATEWAY_KEY` (any
  OpenAI-compatible gateway) or `OPENROUTER_API_KEY`.

A serve binary listens on `0.0.0.0`, on `PORT` (default 3000). Its state goes
to the SQLite file named by `DATABASE_URL`, else to `SERVE_DATA_DIR`, else to
`.serve/` in the working directory, or, with `start --store s3://bucket/prefix`,
to a bucket that a daemon on another machine can take over (see
[Keep the data in a bucket](/framework/serve/#keep-the-data-in-a-bucket)). Its
wire API has no authentication, so run
it behind a host that authenticates requests. Amazon Bedrock AgentCore is the
one supported deployment target; see
[Serve on AgentCore](/framework/serve-agentcore/).

Keep a plain Framework binary when the agent is embedded in an application
that already owns its API, authentication, and storage.

To move to a managed, multi-tenant runtime instead, see
[Moving to Platform or Cloud](/framework/moving-to-platform/).
