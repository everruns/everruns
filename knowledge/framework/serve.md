---
type: Decision
title: "serve: Experimental App Framework and Hosting Contract"
description: "Why the experimental serve crates pair Topcoat's API shape with eve's hosting model, serve the everruns server's /v1 API, stay a thin layer over everruns::Engine, and what is still open."
tags:
  - everruns
  - framework
  - serve
  - experimental
---

# serve: Experimental App Framework and Hosting Contract

**Status: experimental proof of concept.** The code is in `crates/serve`,
`crates/serve-macros`, `crates/serve-build`, and `examples/serve/`. The
crates publish at the platform version as `everruns-serve`,
`everruns-serve-macros` and `everruns-serve-build`, because crates.io `serve`
is taken. The library keeps the import name `serve`, since the macros expand
to `::serve::…` paths. They carry no compatibility promise and stay outside
[API Stability](api-stability.md).

## Problem

The everruns runtime already has agents, sessions, typed tools and a durable
canonical event log. What it lacks for "write an agent, ship it" is
discovery, a declaration of what an app needs, and a contract with whatever
hosts it. Today every application hand-assembles an `Engine`. After a restart
it must rebuild the agent's behavior itself and call `Engine::attach`, because
closures and drivers cannot be serialized.

## Decision

Take the **API shape** from Topcoat (tokio-rs) and the **hosting model** from
Vercel's eve. Build both on the existing `everruns` facade, with no new
runtime.

- **Topcoat's API shape.** An app is `serve::start(App::builder().discover().build())`
  plus attribute macros: `#[agent]`, `#[tool]`, `#[channel]`, `#[schedule]`,
  `#[connection]` and `#[eval]`. Each piece reaches what it needs through one
  `&Cx`, rather than having it threaded through a builder. Topcoat calls this
  locality of behavior.
- **eve's hosting model.** The file layout says what a piece is. The build
  declares what it needs in a manifest, and the host provides it: cron
  entries, webhook routes, secrets, storage, the sandbox and the model
  gateway.
- **The everruns server's wire API.** serve's HTTP API is a subset of the
  everruns server's `/v1` session API, with matching shapes: `Session`,
  `Message`, the canonical event envelope, `/sse` (`since_id` or
  `after_sequence`, `connected`, ids on durable events only) and `/events`,
  cancel, and `/question-answers`. The official SDK, the CLI and the UI chat
  view can therefore drive a serve app, and one client serves both. It is
  served the same way in `dev`, when self-hosted, and when hosted. eve was the
  reference for ergonomics only; its wire shape (`{input}` bodies,
  `Last-Event-ID` resume over a host-authored log) was removed without a
  compatibility layer.
- **Each agent has the Agent Execution API.** `/v1/channels/{agent}` is the
  agent base URL from [Agent Execution API](../integrations/agent-execution-api.md),
  with the wire types shared through `everruns::execution_api`. Its routes
  are the `/v1/sessions` handlers behind a check that the session runs that
  agent, so another agent's URL sees a 404, never a session it does not own.
- **A thin layer over `everruns::Engine`.** The durable log is the engine's
  (`Session::events_after` / `events_from`); serve authors no events. Tools
  are `FunctionTool::with_context`, approvals are the runtime's per-tool gate
  (`needs_approval` + durable approval commands), questions are the built-in
  `ask_user` capability. serve's own SQLite keeps only the session catalog
  (agent, build, title, tags, metadata) and the channel host's state
  (thread bindings, seen message keys, pending replies).
- **Channels are the shared channel host.** `#[channel]` drivers run on
  `everruns::channels::ChannelHost`, the runtime the Framework and the server
  use, so retries, thread binding, streamed replies and restart recovery
  behave the same everywhere. See [Channels](../integrations/channels.md).

## Consequences and choices

- **Link-time discovery.** Rust cannot scan the filesystem at runtime, so the
  macros register items with `inventory` and `serve-build` embeds `agent/**`
  at build time. The layout is a convention, and `discover()` warns about
  drift instead of failing. Discovery collects every error in the app before
  reporting, the way a compiler does, instead of stopping at the first.
- **The binary produces the manifest.** Only the linked binary knows what was
  registered, so the manifest comes from `app manifest`, not from `build.rs`.
  the declaration hash covers the manifest and embedded assets. Durable execution
  additionally pins the executable and the frozen live asset snapshot.
- **The binary is the behavior.** Registered Engine builds pin the executable,
  manifest and frozen instructions/assets. Secret names are captured during that
  app's discovery; unrelated declarations cannot change its build hash. A session
  retains its Agent/Harness/Environment references. The catalog is rebuildable,
  cannot rebind behavior or grant access through another agent's URL, and history
  stays readable when the retained executable is unavailable. Activation suspends
  rather than silently falling back to the current build.
- **Resume by cursor, not by token.** Unlike eve's continuation token, a
  client resumes with the server's cursors: `since_id` (an event id) or
  `after_sequence` (a durable sequence). Both address the engine's canonical
  log, which survives restarts with dense sequences.
- **Apps name models; hosts own keys.** `provider/model` strings resolve
  through a gateway. Without one, `dev` and evals fall back to per-agent
  simulator scripts, so examples and evals run offline and deterministically.
- **Approvals and questions are the runtime's.** Coherent Engine profiles persist
  pending calls and resolve them with exact-turn, deduplicated authority commands.
  Batch validation precedes every write; omitted approval decisions deny and
  mixed AG-UI resume batches commit atomically. The model receives the canonical
  decision/error. Pending requests survive frontend or executor loss without
  replaying approval callbacks. Native persistent serve starts SQLite's independent
  authority supervisor; DO placement relies on stored alarms. Legacy local-store
  integrations retain their older re-parking behavior.
- **App schedules survive a restart.** `dev` and `start` record each
  `#[schedule]` occurrence before running it. On boot, an occurrence that fell
  due while the host was down runs once, right away; several missed
  occurrences collapse into that one run. The first boot only records a
  starting point. See `crates/serve/src/scheduler.rs`.
- **A daemon can keep its data in a bucket.** `start --store s3://bucket/prefix`
  (actor-based design step 5, storage option C) keeps the data directory as
  a local working copy and the bucket as the truth: one lease per daemon,
  taken with conditional writes; a manifest written only by conditional
  update, so it fences out a daemon that was taken over; SQLite files copied
  by changed pages, other files (the workspace) whole by content hash. A
  change ships within the copy interval (200 ms) plus one upload, not before
  it is acknowledged; resume covers the turn a crash cut off. One lease per
  daemon, not per agent: several daemons on one bucket would need per-agent
  leases and per-agent layout, which waits for a real need. The data
  directory's absolute path must match across machines, because a session's
  workspace binding records it. See `crates/serve/src/bucket.rs`.
- **Subagents are tools.** `#[agent(sub)]` becomes `ask_<name>` on the other
  agents and runs a child session on the same engine. The child's tool
  activity is reported as `tool.progress` of the parent call, and its
  approvals and questions wait on the parent session.

- **AG-UI is built in, not a `#[channel]`.** A channel driver answers a
  webhook and delivers the reply later; AG-UI streams the reply in the
  response. So the `ag-ui` feature adds `POST /v1/channels/{agent}/ag-ui` beside the
  channel routes, a thin layer over the facade's `Session::ag_ui_with`. Its
  `InterruptSource` reads serve's parked approvals and questions, so one
  responder serves both APIs. See [AG-UI Channel](../integrations/ag-ui.md#serve).

- **Auth is the server's verifier, opt-in.** With no method configured the
  API stays open, as before. A method list (static keys, OIDC/JWKS, OAuth 2.0
  introspection, in the server's channel auth config shape) from
  `ServerBuilder::auth`, `[auth]` in `serve.toml` or `SERVE_API_KEYS` puts a
  bearer check in front of every agent route, using
  `everruns_core::channel_auth`, the verifier the server's channels use. The
  card, the A2A card (which then lists the methods as A2A security schemes,
  named as the server's card names them), the voice page and messaging-channel
  webhooks (which carry platform signatures, not a serve credential) stay
  public. `eval --against` sends a bearer credential from `SERVE_CREDENTIAL`. It
  authenticates only: callers are not isolated from each other's sessions.
  See `crates/serve/src/auth.rs`.

- **Hosting targets are sibling crates.** A platform contract (AgentCore
  Runtime's `/ping`, `/invocations` and port 8080 first) lives in its own crate,
  `everruns-serve-agentcore`, built on the public `serve::Server` seam: the same
  boot as `start`, the `/v1` router, a busy signal and the AG-UI run. serve
  stays platform-neutral, and the target binary hands every other command back
  to `serve::start`. `/ping` reports busy only while a turn runs, not while it
  waits on a person, so a parked session can go idle instead of billing until
  its maximum lifetime.
- **AgentCore boots lazily and trusts the microVM.** Session storage is mounted
  only once an invocation arrives, so the target answers `/ping` without
  storage and opens the SQLite log on the first other request
  (`/mnt/workspace/.serve` when mounted, else a warned-about temp dir). For
  `sandbox = "microvm"` it supplies a host shell rooted at the session
  workspace through `ServerBuilder::microvm`: the per-session microVM is the
  isolation boundary, and stacking bashkit inside it would only remove real
  tools.
- **celld uses one DO authority per Framework session.** The native adapter selects
  `ObjectHosting` beneath the ordinary serve Engine and authenticates `/execute`
  separately from the user API. Container restart reconstructs retained behavior;
  it does not restore canonical snapshots or replay HTTP requests. The isolate
  example imports the same Framework Agent/Engine Wasm application as Cloudflare.
  Container disk remains a separate workspace concern. Untrusted OS commands need
  an explicit isolated sandbox adapter protecting Engine credentials and other
  sessions; a directory or the executor container alone is insufficient.

Explicit external hosting refuses channels, schedules and child-agent declarations
before listening until their required independent host services are provided.
Native serve retains its existing channel/schedule services; selecting a session
profile alone never establishes cross-object inbox/outbox durability. See
[Engine-owned sessions](engine-durable-sessions.md) for the integration contract
and [DO qualification](durable-object-native.md) for tested failure domains.

## Rejected

- **Runtime filesystem scanning, eve-style.** Not possible for compiled Rust,
  and it would make behavior depend on the deploy directory instead of the
  binary.
- **A manifest-only shared worker.** This would be denser, but custom tools
  would then be limited to Wasm or MCP. One binary per app matches eve and
  keeps tools as plain Rust.
- **Putting the macros in `everruns`.** They stay in separate crates, marked
  experimental, until the shape settles.

## Open questions

- Should serve graduate into an `everruns-app` crate, or become part of
  `everruns` itself?
- Per-build routing and draining in a real host. The PoC defines the contract
  (`build_id`, `409` with `x-serve-build`) but does no routing.
- Cross-object child-agent services and parent wait/cancel aggregation; external
  profiles currently refuse that declaration rather than promise durable delivery.
- Per-caller session isolation once auth is on, and which further server
  routes (message listing, `tool-results`) a
  serve app should answer so every client works unchanged.
- `#[memoize]` scoped to a turn, and Postgres or NATS adapters for `start`.
- A copy to the bucket before each commit is acknowledged, instead of every
  copy interval, and per-agent leases so several daemons share one bucket.
- A `cargo serve` wrapper for `build` (binary, `manifest.json`, OCI image) and
  `deploy`.

## See also

- [Application API Boundaries](application-api.md)
- [Framework Harnesses](harnesses.md)
- `crates/serve/README.md`, `crates/serve/docs/`
