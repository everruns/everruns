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
`crates/serve-macros`, `crates/serve-build`, and `examples/serve/`. It is
unpublished (`publish = false`) and outside [API Stability](api-stability.md).

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
- **A thin layer over `everruns::Engine`.** The durable log is the engine's
  (`Session::events_after` / `events_from`); serve authors no events. Tools
  are `FunctionTool::with_context`, approvals are the runtime's per-tool gate
  (`needs_approval` + `AgentBuilder::approver`), questions are the built-in
  `ask_user` capability. serve's own SQLite keeps only the session catalog
  (agent, build, title, tags, metadata, delivery target) and channel threads.

## Consequences and choices

- **Link-time discovery.** Rust cannot scan the filesystem at runtime, so the
  macros register items with `inventory` and `serve-build` embeds `agent/**`
  at build time. The layout is a convention, and `discover()` warns about
  drift instead of failing. Discovery collects every error in the app before
  reporting, the way a compiler does, instead of stopping at the first.
- **The binary produces the manifest.** Only the linked binary knows what was
  registered, so the manifest comes from `app manifest`, not from `build.rs`.
  `build_id` hashes the manifest plus the embedded assets, so a prompt edit is
  a new build.
- **The binary is the behavior.** A session records the build it started on.
  Resuming a session means rebuilding its agent from the same registrations,
  then calling `Engine::attach` and `Engine::resume` over the everruns local
  store. `start` answers `409` with the owning build for a foreign session, so
  a router can send it back to that build; `dev` resumes it anyway. This
  removes the existing need for applications to reattach behavior after a
  restart.
- **Resume by cursor, not by token.** Unlike eve's continuation token, a
  client resumes with the server's cursors: `since_id` (an event id) or
  `after_sequence` (a durable sequence). Both address the engine's canonical
  log, which survives restarts with dense sequences.
- **Apps name models; hosts own keys.** `provider/model` strings resolve
  through a gateway. Without one, `dev` and evals fall back to per-agent
  simulator scripts, so examples and evals run offline and deterministically.
- **Approvals and questions are the runtime's.** A predicate over the
  macro-generated argument struct becomes `FunctionTool::needs_approval`;
  serve's approver parks the call, keyed by session and tool call id, until
  `POST …/approvals/{tool_call_id}` (serve-only; the server has no such route)
  answers it. `ask_user` questions are parked the same way and answered
  through the server's `/question-answers`. There is no canonical event for a
  pending request, so the session lists them (`pending_approvals`,
  `pending_questions`) and reports `waitingfortoolresults`. The PoC keeps
  them in memory only, and the runtime passes the model no deny note.
- **Subagents are tools.** `#[agent(sub)]` becomes `ask_<name>` on the other
  agents and runs a child session on the same engine. The child's tool
  activity is reported as `tool.progress` of the parent call, and its
  approvals and questions wait on the parent session.

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
- Durable approvals and `ask_user` across restarts, and a deny note the
  model can read.
- Which further server routes (auth, message listing, `tool-results`) a
  serve app should answer so every client works unchanged.
- `#[memoize]` scoped to a turn, and Postgres or NATS adapters for `start`.
- A `cargo serve` wrapper for `build` (binary, `manifest.json`, OCI image) and
  `deploy`.

## See also

- [Application API Boundaries](application-api.md)
- [Framework Harnesses](harnesses.md)
- `crates/serve/README.md`, `crates/serve/docs/`
