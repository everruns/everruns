---
title: Serve (experimental)
description: Build a hosted agent app with attribute macros, file-layout discovery and a manifest, served over a subset of the Everruns server /v1 API.
---

> **Experimental.** serve is a proof of concept. Its APIs will change and it
> has no compatibility promise.

**serve** is an application framework on top of the Everruns Framework. You
write an app as a set of annotated functions, the build declares what the app
needs from its host, and the binary serves the same `/v1` session API as the
Everruns server, so the SDKs, CLI and UI chat view can drive it.

serve adds no runtime of its own. Agents, sessions, tools, approvals,
[`ask_user`](/framework/ask-user/), skills and the durable event log all come
from the `everruns` crate: serve is a thin layer over
[`Engine`](/framework/architecture/).

![Serve request map: the /v1 session routes, channel webhooks, the AG-UI and A2A routes, AgentCore's /invocations and /ws, and in-process schedules all reach one serve host, which runs sessions, approvals and ask_user questions on one everruns::Engine backed by a local SQLite session log.](./serve-routes.svg)

```rust ignore
use serve::prelude::*;

#[agent]
fn analyst() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")    // resolved by the host's model gateway
        .instructions(md!("instructions.md"))  // agent/instructions.md, hot-reloads in dev
        .build()
}

/// Run a read-only SQL query against the warehouse.
#[tool(needs_approval = |a: &RunSql| scan_gb(&a.sql) > 50.0)]
async fn run_sql(cx: &Cx, sql: String) -> Result<Rows> {
    let wh = cx.connection::<Warehouse>()?;   // credentials never reach the model
    cx.progress("querying…").await;
    Ok(wh.query(&sql)?.truncate(500))
}

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(App::builder().discover().build()).await
}
```

## Install

```sh
cargo add everruns-serve tokio --features tokio/full
cargo add --build everruns-serve-build
```

The package is `everruns-serve` and the library is imported as `serve`. Add a
`build.rs` that calls `serve_build::embed()` so `agent/**` and `serve.toml` are
compiled into the binary.

## Try it

Both examples run offline against a scripted simulator, so no keys or network
are needed. From a checkout of the repository:

```sh
cargo run -p serve-example-revenue-analyst            # dev server on :3000 with a live console
cargo run -p serve-example-revenue-analyst -- eval    # run the evals in-process
cargo run -p serve-example-revenue-analyst -- manifest
```

Then talk to it with the same requests you would send an Everruns server:

```sh
ID=$(curl -s localhost:3000/v1/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"message":{"content":[{"type":"text","text":"What was revenue last week?"}]}}'
curl -N "localhost:3000/v1/sessions/$ID/sse?after_sequence=0"
```

To use a real model, set `OPENROUTER_API_KEY`, or point `SERVE_GATEWAY_URL`
and `SERVE_GATEWAY_KEY` at any OpenAI-compatible gateway. With serve's
`bedrock` feature, `bedrock/<model-id>` models (for example
`bedrock/us.anthropic.claude-sonnet-4-6`) call Amazon Bedrock on the AWS
default credential chain whenever `AWS_REGION` is set.

| Example | Shows |
| --- | --- |
| [Hello app](https://github.com/everruns/everruns/tree/main/examples/serve/hello) | The smallest app: one agent, one tool, one eval. |
| [Revenue Analyst](https://github.com/everruns/everruns/tree/main/examples/serve/revenue-analyst) | A tool with approvals, a skill, Slack, a schedule, MCP and typed connections, a subagent, evals and the Bashkit sandbox. |
| [AG-UI app](https://github.com/everruns/everruns/tree/main/examples/serve/ag-ui) | An agent streamed to `@ag-ui/client` and CopilotKit, with an approval as an interrupt. |
| [AgentCore app](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore) | The same kind of app packaged for Amazon Bedrock AgentCore Runtime. |
| [AgentCore workspace app](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore-workspace) | An AgentCore workspace agent with a shell in the microVM and an approval. |
| [A2A example](https://github.com/everruns/everruns/tree/main/examples/serve/a2a) | Two agents over A2A: a served `researcher`, and an `everruns` agent that delegates to it. |
| [Voice example](https://github.com/everruns/everruns/tree/main/examples/serve/voice) | A hotel front desk people call from the browser, with a greeting and a speaking style. |

## Project layout

The file layout says what each piece is:

```text
revenue-analyst/
  build.rs                 # serve_build::embed()
  serve.toml               # name, sandbox, extra secrets
  agent/
    instructions.md        # always-on prompt
    skills/sql-style/SKILL.md
  src/
    main.rs                # serve::assets!() + serve::start(...)
    agent.rs               # #[agent]
    tools/run_sql.rs       # #[tool]
    channels/slack.rs      # #[channel]
    schedules/weekly.rs    # #[schedule]
    connections/linear.rs  # #[connection]
    subagents/reviewer.rs  # #[agent(sub)]
  evals/revenue.rs         # #[eval]
```

The macros register each item at link time and `discover()` collects them;
`build.rs` embeds `agent/**` and `serve.toml` into the binary. `discover()`
reports every problem in the app at once, such as duplicate names, a bad cron
expression or a filter naming an unknown tool.

## The pieces

| Macro | On | Becomes |
| --- | --- | --- |
| `#[agent]`, `#[agent(default)]`, `#[agent(sub)]` | `fn() -> Agent` | An agent. New sessions use the default one; a subagent becomes an `ask_<name>` tool on the others. |
| `#[tool]`, `#[tool(needs_approval)]`, `#[tool(needs_approval = \|a: &Args\| …)]` | `async fn([cx: &Cx,] args…) -> Result<T>` | A tool named after the function, described by its doc comment. It also generates an arguments struct (`run_sql` → `RunSql`) with a JSON Schema. |
| `#[channel]` | `fn() -> impl Into<Channel>` (a driver such as `Slack` or `Webhook`) | `POST /v1/channels/{name}`. Each external thread maps to one session, and replies are delivered back while the turn runs. `agent = "…"` picks the agent. |
| `#[schedule("0 9 * * MON")]` | `async fn(&Cx) -> Result` | A cron entry. |
| `#[connection]` | `fn() -> T` or `fn() -> Result<T>` | A value tools read with `cx.connection::<T>()`. An `McpServer` connection is attached to every agent. |
| `#[eval]` | `async fn(&mut EvalCx) -> Result` | A conversation with assertions, run in-process or against a deployment. |

`Cx` is the one context type. Inside a tool it wraps the Framework's
`ToolCallContext` (session, turn and tool call ids, and `progress`) and adds
typed connections, secrets and `start_session`. Approval rules are declared on
the tool and enforced by the runtime.

## Commands

The commands are built into the app binary, because only the linked binary
knows what it registered.

| Command | What it does |
| --- | --- |
| `dev` (default) | Serves the API with local SQLite under `.serve/` and a live console. Models fall back to the simulator, and Markdown prompts and skills hot-reload. |
| `start` | Production mode. Every model must route through a gateway, and every declared secret must be set. With `--store s3://bucket/prefix` (or `SERVE_STORE`) the data lives in a bucket; see [Keep the data in a bucket](#keep-the-data-in-a-bucket). |
| `manifest` | Prints the host contract as JSON: agents, models, tool schemas, skills, channels, cron entries, secrets, sandbox, evals and a build id. |
| `eval [--against URL]` | Runs the evals in-process, or against a running deployment as a gate before promotion. |
| `deploy` | Prints what a host would provision from the manifest. The one supported deployment target is [Amazon Bedrock AgentCore](/framework/serve-agentcore/). |

## Keep the data in a bucket

`start --store s3://bucket/prefix` keeps a daemon's data in an S3 bucket, or
in any S3-compatible store that supports conditional writes (R2, MinIO, GCS,
Tigris, SeaweedFS). The data directory becomes a local working copy:

- At boot the daemon takes the bucket's lease, then rebuilds the data
  directory from the bucket. Files the bucket does not have are removed.
- While it runs, every change is copied to the bucket within about a fifth of
  a second: databases by the pages that changed, other files (the agents'
  workspace) whole.
- A daemon started on another machine with the same `--store` waits for the
  old one's lease to expire (15 seconds), rebuilds the data directory, and
  resumes the turns the old one was running. While the old daemon is still
  alive and renewing, the new one refuses to start. If the old one wakes up
  after it was replaced, its next write fails and it stops.
- A clean shutdown (Ctrl+C) copies what is left and releases the lease, so
  the next daemon starts at once.

Credentials, region and endpoint come from the standard `AWS_*` environment
variables. For an S3-compatible store set `AWS_ENDPOINT`, and `AWS_ALLOW_HTTP=true`
for a plain-HTTP endpoint. Keep the data directory at the same absolute path
on every machine (`SERVE_DATA_DIR`), because a session's workspace records it.

A crash can lose the changes of its last fifth of a second. One daemon holds
a bucket prefix at a time; give each daemon its own prefix.

## Wire API

A serve app speaks a subset of the Everruns server `/v1` API, with the same
request and response bodies:

- `POST /v1/sessions` and `GET /v1/sessions/{id}`
- `POST /v1/sessions/{id}/messages`, which also steers an active turn
- `POST /v1/sessions/{id}/cancel`
- `GET /v1/sessions/{id}/sse` (resume with `since_id` or `after_sequence`) and `GET /v1/sessions/{id}/events`
- `POST /v1/sessions/{id}/question-answers`, for [`ask_user`](/framework/ask-user/) questions
- `POST /v1/sessions/{id}/tool-approvals`, to allow or reject pending tool calls

It adds a few routes of its own: `GET /health`, `GET /v1/agent` (the agent
card), channel webhooks, the [AG-UI](#ag-ui-and-copilotkit), [A2A](#a2a) and [voice](#voice) routes, and `POST /v1/sessions/{id}/approvals/{tool_call_id}`
to approve or deny one pending tool call. Errors are `application/problem+json`.

### One agent's API

Every top-level agent also has its own base URL, `/v1/channels/{agent}`, with
the same session routes rooted under it: `GET` returns the agent's card,
`POST /v1/channels/{agent}/sessions` starts a session that runs that agent,
and `/v1/channels/{agent}/sessions/{id}/…` carries messages, events,
cancellation, questions and approvals. A session is reachable only under its
own agent's URL. This is the API an Everruns agent exposes through an API
channel, so code written against one agent works against either host.

### Authentication

By default the wire API has no authentication and no organizations: run it
locally, or behind a host that authenticates requests. Configure one or more
methods and every agent route requires `Authorization: Bearer <credential>`;
anything else gets `401` with `WWW-Authenticate: Bearer`. The methods are the
ones an Everruns API channel accepts, checked by the same verifier:

- **API keys.** Static keys, compared in constant time. Set them in
  `SERVE_API_KEYS`, comma separated.
- **OpenID Connect.** JWTs from an issuer, checked against the keys its
  discovery document publishes, with a required audience.
- **OAuth 2.0 introspection.** Opaque tokens checked at an RFC 7662 endpoint.

Token methods take the same requirements as a channel: audiences, scopes,
subjects, groups, email domains and exact claim values. Methods add up from
code, `serve.toml` and the environment:

```rust
use serve::auth::AuthMethod;

let server = serve::Server::builder(app, serve::Mode::Start)
    .auth([AuthMethod::oidc("https://login.example.com", ["support-agent"])])
    .build()?;
```

```toml
# serve.toml
[[auth.methods]]
mode = "oidc"
provider = { type = "oidc", issuer = "https://login.example.com" }
requirements = { audiences = ["support-agent"], scopes = ["agent:call"] }
```

```sh
SERVE_API_KEYS=key-for-ci,key-for-the-web-app ./revenue-analyst start
```

`serve.toml` also takes `api_keys = [...]` under `[auth]`, but it is embedded
in the binary, so keep real keys in the environment.

A few routes stay public so clients can find the agent: `GET /health`, each
agent's card at `GET /v1/channels/{agent}` (its `auth` list names the accepted
methods), the A2A Agent Card and the voice test page. Messaging channel
webhooks stay public too, because Slack and webhook callers cannot send a serve
credential; each channel verifies its platform's own signature. Auth does not
separate callers: every authenticated caller can reach every session.

## AG-UI and CopilotKit

With the `ag-ui` feature, every top-level agent also serves
[AG-UI](https://docs.ag-ui.com) 1.0 clients such as CopilotKit and
`@ag-ui/client`:

```sh
cargo add everruns-serve --features ag-ui
```

The route is `POST /v1/channels/{agent}/ag-ui`, the same shape as an Everruns
endpoint's AG-UI route, so a front end moves between a local serve app and
Everruns by base URL and id alone:

```ts
import { HttpAgent } from "@ag-ui/client";

const agent = new HttpAgent({ url: "http://localhost:3000/v1/channels/analyst/ag-ui" });
```

Each AG-UI `threadId` maps to one session, which survives a restart. A
thread's first run records the input's earlier user and assistant messages as
the new session's history; after that, a run sends only the input's last user
message. Each run streams the turn as AG-UI events,
with reasoning, token usage and errors visible. A tool approval or an
[`ask_user`](/framework/ask-user/) question ends the run with an interrupt
(`tool_approval` or `everruns.ask_user`), and the next run's `resume` entries
answer it and stream the rest of the turn. These are the same pending requests
the approvals and `question-answers` routes answer, so either API can resolve
them. Input that is not a valid `RunAgentInput` gets `400`, and an unknown
agent `404`. The agent card lists each agent's route under `ag_ui`. See
[Serve AG-UI](/framework/ag-ui/) for the event and interrupt shapes.

## A2A

With the `a2a` feature, every top-level agent also serves other agents over
[A2A](https://a2a-protocol.org) 1.0 JSON-RPC:

```sh
cargo add everruns-serve --features a2a
```

The endpoint is `POST /v1/channels/{agent}/a2a` and its Agent Card is
`GET /v1/channels/{agent}/a2a/.well-known/agent-card.json`, the shape of an Everruns
A2A endpoint. Any A2A 1.0 client works, including the official `a2a` CLI and
another Everruns agent's `a2a_agent_delegation` capability:

```sh
a2a send -a http://localhost:3000/v1/channels/researcher/a2a/.well-known/agent-card.json "Tide pools"
```

Each A2A `contextId` maps to one session, which survives a restart; each task
is one turn. A task streams `working`, then the final reply as a `response`
artifact, then `completed` (or `failed`). `GetTask`, `ListTasks`, `CancelTask`
and `SubscribeToTask` come from the A2A Rust SDK's request handler; tasks are
kept in memory, so old task ids are forgotten on restart while their context
continues. Requests need the `A2A-Version: 1.0` header. A pending approval or
`ask_user` question keeps the task `working` until the routes above answer it.
The agent card lists each agent's endpoint under `a2a`.
[Framework A2A](/framework/a2a/) covers serving and calling A2A agents, and
[A2A example](https://github.com/everruns/everruns/tree/main/examples/serve/a2a)
shows a served agent and a second agent that delegates to it.

Sessions survive a restart: the binary rebuilds each agent and resumes the
session from the local store. A turn the old process left waiting on an
approval or an `ask_user` question waits again: the call runs again, so the
request is pending once more under the same tool call id, and answering it
finishes the turn. A turn cut off while a tool executed is not re-run. A
session pinned to a different build gets
`409 Conflict` with an `x-serve-build` header, so a host can route it to the
build that owns it.


## Voice

With the `voice` feature, every top-level agent also takes browser calls
through a [voice channel](/framework/voice/):

```sh
cargo add everruns-serve --features voice
```

`POST /v1/channels/{agent}/voice/calls` takes the browser's WebRTC offer and
answers the SDP answer, the call id and the session id;
`POST /v1/channels/{agent}/voice/calls/{call_id}/end` hangs up, and
`GET /v1/channels/{agent}/voice` is a test page with a **Start call** button.
Each utterance is a user message on an ordinary session, so a call can continue
a typed conversation. The optional `[voice]` section of `serve.toml` sets the
voice, greeting and speaking style for every agent. Speech uses OpenAI with
`OPENAI_API_KEY`; without it, `dev` and `eval` use the offline simulator and
`start` refuses calls. The agent card lists each agent's route under
`voice.endpoints`. See [Build a voice agent](/framework/voice/).
## Limitations

- After a restart, an approval answered "always" before it is asked again, and a
  request parked by a subagent is lost.
- A2A tasks are held in memory, and A2A 0.3 clients are not served.
- A deny note is not passed to the model.
- There is no generic OCI build or Postgres or NATS adapter. [AgentCore](/framework/serve-agentcore/) is the one supported deployment target.
- The server's agent, harness, workspace and tool-result routes are not served.

## Amazon Bedrock AgentCore

[`everruns-serve-agentcore`](https://docs.rs/everruns-serve-agentcore) runs a
serve app on AgentCore Runtime. Replace `serve::start` with
`serve_agentcore::start` in `main`. With no command, the binary then serves
AgentCore's contract on port 8080: `GET /ping` and `POST /invocations`, which
takes an AG-UI `RunAgentInput` or `{"prompt": "..."}` and streams AG-UI
events. serve's own commands keep working. See
[Serve on AgentCore](/framework/serve-agentcore/) for deployment, persistence on
session storage, tools and models.

## celld

[`everruns-serve-celld`](https://docs.rs/everruns-serve-celld) runs a serve app
durably on [celld](https://github.com/denoland/celld), self-hosted Durable
Objects. Replace `serve::start` with `serve_celld::start`; the binary then runs
in a container that a Durable Object supervises, and the object keeps a snapshot
and a request journal so a lost container is restored and its interrupted turn
replayed. See [Serve on celld](/framework/serve-celld/).

The full guide, wire reference and hosting contract live next to the crate in
[serve reference guides](https://github.com/everruns/everruns/tree/main/crates/serve/docs).

## File-based agents

Load and export the shared portable package format, including initial files and
complete skill directories. See [Define agents as files](/how-to/define-agents-as-files/)
for validation, diffs, model bindings and runnable examples.
