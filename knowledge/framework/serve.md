---
type: Decision
title: "serve: Experimental App Framework and Hosting Contract"
description: "Why the experimental serve crates pair Topcoat's API shape with eve's hosting model on top of the everruns runtime, and what is still open."
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
  gateway. The same `/v1` wire API is served in `dev`, when self-hosted, and
  when hosted.

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
  client resumes the SSE stream with `Last-Event-ID`, which is a position in
  the session's log. The PoC mirrors the reviewed everruns events, plus the
  host's own events, into a per-session SQLite log. A production host should
  serve the canonical log directly.
- **Apps name models; hosts own keys.** `provider/model` strings resolve
  through a gateway. Without one, `dev` and evals fall back to per-agent
  simulator scripts, so examples and evals run offline and deterministically.
- **Approvals are a tool gate in serve, not a runtime feature.** A predicate
  over the macro-generated argument struct decides whether a call pauses for
  `POST …/approvals/{id}`. This PoC keeps pending approvals in memory only.
- **Subagents are tools.** `#[agent(sub)]` becomes `ask_<name>` on the other
  agents and runs in an ephemeral session. Its events report on the parent's
  stream.

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
- Durable approvals and `ask_user` across restarts.
- `#[memoize]` scoped to a turn, and Postgres or NATS adapters for `start`.
- A `cargo serve` wrapper for `build` (binary, `manifest.json`, OCI image) and
  `deploy`.

## See also

- [Application API Boundaries](application-api.md)
- [Framework Harnesses](harnesses.md)
- `crates/serve/README.md`, `crates/serve/docs/`
