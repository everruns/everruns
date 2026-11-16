---
title: Deploying a Framework App
description: "Ship a Framework app as a binary or container: persistent storage for its data directory, provider keys from the environment, and when to use serve instead."
---

A Framework app is your own Rust binary. The `everruns` crate needs no
database, message broker, or Everruns server, so deploying it is deploying that
binary: build it and give it provider credentials. If sessions must outlive
the process, select durable session hosting and preserve its storage and
registered Agent builds. A persistent directory alone is not an independent
recovery mechanism.

## Choose where state lives

| Engine setup | State | Deploy with |
|---|---|---|
| `Engine::new()` | In memory, lost when the process exits | Any binary or container. Nothing to persist |
| Engine with `hosting::SqliteHosting` | Transactional session authority, canonical events, admission receipts and wakeup obligations | Persistent SQLite storage, retained Agent builds and Environment factories, and a running native supervisor |
| An agent built with legacy `.local(LocalConfig::new(dir))` | Local events, session catalog, and SQLite task and schedule state under `dir`; agent files under `dir/workspace` unless `.workspace(path)` sets another root | A persistent volume and application-driven reattachment and recovery |

`SqliteHosting` and `LocalConfig` need the `local` feature. New durable services
should select `SqliteHosting` on `Engine::builder()`, register immutable
`AgentBuild` identities and start `SqliteHosting::supervise` at startup,
independently of incoming requests. Workspace files, compute and secrets need
explicit services with their own persistence guarantees. See
[session hosting](/framework/hosting/) and
[Sessions](/framework/sessions/#persistence) for the recovery boundaries.

### Existing LocalConfig deployments

`AgentBuilder::local` is deprecated. The following guidance and example apply
to that legacy lane; use [legacy session cutover](/framework/sessions/#moving-legacy-sessions-to-sqlitehosting)
to move existing sessions to coherent Engine hosting.

For a legacy deployment that keeps local state:

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

## Advanced local hosts

Hosts built with `everruns::local::LocalRuntimeBuilder` reconcile orphaned background
executions after constructing their runtime and start periodic maintenance. Recovery
requires executor evidence that ownership is lost. A missing owner process leaves the
execution outcome unverified; interrupted commands are never replayed.

When composing `LocalBackends` directly, retain its recovery-wrapped task registry,
bind `task_recovery` to the runtime's task wake observer, then call `recover_all`
and `start_maintenance`. Recovery wakes are delivered at least once across crashes;
a durable observer must deduplicate by session, task ID and terminal attempt.

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

A legacy `LocalConfig` service entry point:

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
    .model("gpt-6.1-sol")
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

A volatile Engine needs no state volume. A container using `SqliteHosting`,
`LocalConfig` or persistent workspace services needs storage that survives
replacement.

## When to use serve instead

[serve](/framework/serve/) is an experimental application framework on top of
the Framework. Use it instead of a hand-written binary when you want:

- the Everruns `/v1` session API, so the SDKs, CLI, and UI chat view can drive
  your agents, plus AG-UI and A2A endpoints;
- a manifest of agents, tools, secrets, and models that a host can provision
  from;
- a `start` command that requires every declared secret, and that fails a
  model with no real route instead of falling back to the simulator. Hosts
  configure `OPENAI_API_KEY` for `openai/…` models, optionally with
  `OPENAI_BASE_URL` for a compatible Responses API endpoint, or use
  `OPENROUTER_API_KEY` for provider-qualified model IDs.

A serve binary listens on `0.0.0.0`, on `PORT` (default 3000). Its state goes
to the SQLite file named by `DATABASE_URL`, else to `SERVE_DATA_DIR`, else to
`.serve/` in the working directory, or, with `start --store s3://bucket/prefix`,
to a bucket that a daemon on another machine can take over (see
[Keep the data in a bucket](/framework/serve/#keep-the-data-in-a-bucket)). Its
wire API is unauthenticated by default. Configure
[serve authentication](/framework/serve/#authentication) or place it behind an
authenticating host before exposing it. See
[Serve on AgentCore](/framework/serve-agentcore/) for Amazon Bedrock AgentCore;
the [celld adapter](/framework/serve-celld/) is a separate source-only reference,
not a published deployment integration.

Keep a plain Framework binary when the agent is embedded in an application
that already owns its API, authentication, and storage.

To move to a managed, multi-tenant runtime instead, see
[Moving to Platform or Cloud](/framework/moving-to-platform/).

## Connect custom code to a control plane

Use an experimental [custom agent worker](/how-to/custom-agent-workers/) when
your binary should own private tools and model access while the self-hosted
Platform owns user sessions, event history and tracing. The
[data-agent walkthrough](/framework/examples/custom-data-agent/) includes
container packaging and a real-model end-to-end smoke test.
