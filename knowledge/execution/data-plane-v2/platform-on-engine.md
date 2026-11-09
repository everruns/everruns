---
type: Specification
title: Platform sessions on the Everruns Engine
description: What it takes to run every platform feature under Engine hosting, so workers and cells share one execution path.
tags:
  - execution
  - data-plane-v2
  - engine
---

# Running platform sessions on the Everruns Engine

Status: approved 2026-11-09; implementation in progress. Part of [design.md](design.md).

## Question

The Engine (`everruns::Engine` with pluggable `SessionHosting`) runs Framework, Durable Object and funout sessions. Platform sessions on workers do not use it. Data plane v2 needs every platform feature to run under Engine hosting.

## Finding: the turn is already shared; only host construction differs

Both paths run the same core `SessionActorDriver` (`crates/core/src/host/session_actor/mod.rs`). That means the same turn loop, the same command reducer, the same journal fold and the same resume logic. The only difference is the `RuntimeHostAdapter` sitting under the driver:

| | Platform (workers today) | Engine |
| --- | --- | --- |
| Host type | `WorkerRuntimeHost` over gRPC `WorkerAdapters` (`crates/worker/src/runtime_host.rs`) | `InProcessRuntime` from `HostBackends` + `HostComposition` (`crates/core/src/host/runtime/mod.rs`) |
| Adapter methods provided | about 40 | about 30, several only as defaults |
| Agent source | Database config per turn (`load_resolved_turn_for_execution`) | Registered code `Agent` per `AgentBuild` |

So the gap is mostly that **the platform's services cannot be handed to the Engine**, not that the Engine lacks the concepts. Every platform service is already a core trait. The Engine builds the runtime itself and accepts only:

- the epoch's `HostBackends`, enriched in `SessionHosting::prepare_activation`;
- `HostServices` on the agent;
- the `Environment`.

Several adapter slots have no field in any of the three.

## Gap by service

| Service | Gap | How it closes |
| --- | --- | --- |
| Model providers, drivers | wiring | `HostServices` providers |
| Workspace files | wiring | `EnvironmentFactory` (in v2, the cell workspace tree from [fs.md](fs.md)) |
| Memories | wiring | `HostBackends.memory_mounts` |
| Compaction checkpoints | none | `HostBackends.compaction_checkpoint_store` |
| Approvals, client tools, questions | wiring | Shared reducer. The platform's extra wait kinds (tool approval, URL elicitation) go into the `waits_on_person` predicate |
| Schedules, triggers | wiring | `schedule_store_factory` via `prepare_activation`. A trigger firing becomes an admission |
| Channels (Slack and others) | wiring | Ingress stays outside the Engine. Outbound tools come through `tool_context_extensions_factory` |
| Platform tools (catalog CLI, saved scripts, SQL DB, Poppy, user MCP) | wiring + small | `tool_context_extensions_factory` also needs the session's resolved capabilities, not just org and session. User MCP also needs the turn's **execution context** (verified input author, participants, allowed grants), which travels with the admitted input; see "Execution context" in design.md |
| Tool augmentation, reactions, tracing, observers | wiring | Existing `HostBackends` fields, `host_listener` |
| Live events and token deltas | wiring | The hosting profile's event log publishes cross-process (in v2: cell SSE plus the delta channel) |
| Subagents and teams | wiring + missing slots | Delegate factory exists. **Session creation authority** and **spawn store** have no slot |
| Sandboxes | missing slots | **Leased resources**, **durable tool results**, **session resource registry** |
| MCP servers, OAuth | missing slot | **Hosted MCP resolver** (prefix lookup) |
| Web tools, egress | blocked | Engine always uses the built-in egress service; there's no way to supply the platform egress (SSRF guard, rate limits) |
| Utility LLM, decisions | blocked | `HostComposition` is fixed inside the Engine |
| Usage, budgets, payment, rate limits | missing slots | `budget_checker` exists on the runtime builder but the Engine never calls it; **payment authority** and **outbound rate limiter** have no slot |
| Images and files by reference | missing slots | **Image resolver**, **file resolver**, **image artifact store** |
| Provider credentials, Agents API store | missing slots | No field |
| Knowledge auto-context | missing hook | No way to add retrieved context to a turn's input |
| Detached long tool calls | missing | The worker wraps the run with a tool detacher. The Engine has no equivalent |
| **Agents from configuration** | **design** | The Engine needs a registered code `Agent`, and a registry miss is a hard error. Closed by step 3: definitions are data the authority keeps and hands over with each activation |

Two Engine rules also stand in the way:

1. **Capability ids can't collide.** Host capabilities may not reuse a Framework capability id (a threat-model rule in `hosting/services.rs`). The platform's own versions of built-ins (filesystem, web fetch) therefore collide.
2. **Wasm limits.** These matter for the Durable Object targets but not for workers or cells.

## Proposal: one platform host bundle, then the platform itself moves onto the Engine

This needs solving regardless of cells. Once it's done, v1 workers and v2 cells run the same code and differ only in where session state lives (PostgreSQL or cell). The `WorkerRuntimeHost` path is then deleted.

1. **Engine hooks** (from branch `claude/project-thread-lnfjd8`, plus session protocol versioning from design.md step 0):
   - Activations carry the agent definition; the Engine hydrates it (cached by agent and revision). An unfinished turn keeps its retained `(agent_revision, protocol_major)` across crash, hand-off and move; only a new turn gets the current one. Native tools and hydration declare a compatibility identity, so an executor serves an old revision only if it still supports it. The session binding pins agent identity and protocol major, not a behavior version. No registry or resolver.
   - Admitted input carries an execution context (who the turn acts for).
   - `SessionEpoch::hand_off`.
   - Committed event batches: the authority assigns sequences at commit, listeners see only committed events, admission during a live epoch goes through the inbox, provider calls are durable attempts with one settlement each, and `started` lives in authority state. This moves funout's write-pipeline gains (23 → about 11 waited writes per turn) into Engine safely, which PostgreSQL hosting (step 4) needs to fit the 8k budgets.
   - Session protocol versioning (major per session, supported range per executor, execution-critical event types as required capabilities).
2. **Trusted host extensions.** A single `HostExtensions` bundle, available only to trusted hosts (not a public Framework API), grouped by concern:
   - **Composition:** egress service, utility LLM, decisions.
   - **Authorities:** budget and payment, session creation, outbound rate limiter.
   - **Stores:** provider credentials, leased resources, durable tool results, session resources, spawn store, image and file resolvers, image artifacts, Agents API store.
   - **Resolvers and hooks:** hosted MCP, knowledge auto-context, tool detacher, wait predicate.

   `InProcessRuntime` forwards each one to the existing `RuntimeHostAdapter` method. The traits already exist, so this is plumbing. The capability collision rule gets one trusted-only exception: a platform host may **replace** a built-in by id, and must declare that it does. Framework apps keep the rule as it is.
3. **Agent definitions as data.** One definition format (agent, harness, capabilities including platform ones, MCP servers, connections, sealed secrets), built from stored config by the control plane and kept by the authority: the cell in v2, PostgreSQL in v1. Hydration into an Engine `Agent` reuses the agent-package mapping (`AgentBuilder::package`). Today's per-turn database read becomes "hand over the session's current definition with the activation"; an edit applies at the next turn.
4. **Workers run the Engine.**
   - Add a PostgreSQL `SessionHosting` over today's `session_actors` and `events` tables. It uses the same reducer the server already runs under its row lock.
   - Workers call `Engine::activate` and supply the bundle from their gRPC adapters.
   - Run the hosting conformance suite against PostgreSQL.
   - Remove `WorkerRuntimeHost`, so there is one platform execution path.

   This is the largest step. It is done when every platform feature runs through the Engine in production.
5. **Cells reuse all of it.** The cell's `SessionHosting` and cell-local adapters supply the same bundle. Data plane v2 then gets every platform feature without per-feature work, and stages 2 and 3 in design.md shrink to the feature-specific state that has to live in the cell: sandbox records, schedules, detached calls.

## Size and order (estimate)

These steps are now part of the single ordered plan, with a bench gate per step, in design.md "Implementation plan" (steps 1 to 4 there). The table below is kept for sizing.

| Step | Size | Depends on |
| --- | --- | --- |
| 1. Engine hooks | medium (committed batches and attempts are the largest part) | none |
| 2. Host extensions | medium (many slots, each a forwarder plus a test) | 1 |
| 3. Agent definitions as data | medium | 1 |
| 4. Workers on the Engine (PostgreSQL hosting) | large | 1, 2, 3 |
| 5. Cells | covered by design.md plan steps 5 and 6 | 2, 3 (not 4) |

Steps 4 and 5 can run in parallel once 2 and 3 land.

## Risks

- **The Engine's surface grows.** Keeping the bundle trusted-only, and grouped rather than spread across new `HostBackends` fields, keeps the Framework API unchanged.
- **Behavior drift during the worker cutover.** Mitigations: run the platform e2e suite and the hosting conformance suite against PostgreSQL, and cut over at deploy time (in-flight turns resume from the journal, as they did in the actor cutover).
- **The Durable Object / Cloudflare / celld publication hold still applies.** No changelog entry or announcement for Engine hosting changes until the user approves one.
