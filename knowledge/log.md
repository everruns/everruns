# Everruns Knowledge Update Log

## 2026-10-06

* **Database utilities have their own crate.** `everruns-db` (a leaf with no
  `everruns-*` dependency) now owns the embedded SQLite wrapper and
  `UpdateField`, taken out of `everruns-durable`. The facade's `local` feature
  and the serve hosts open SQLite through it and no longer compile the durable
  engine; it joins the database-driver guard's owners, and durable's isolation
  guard allows it as durable's one `everruns-*` dependency. See
  [Crate Layout](project/crate-layout.md#only-the-server-durable-and-db-touch-a-database).

* **One turn entry point for server and framework.** The server's
  `AgentRunner` shim is gone: the server persists input, then starts,
  continues and cancels turns through `TurnBackend` on `DurableRunner`, as the
  facade does. The doc-hidden `TurnInput::Persisted` gave way to two documented
  inputs every backend serves, `StoredMessage` (with an optional `TurnScope` on
  the request) and `RecordedToolResults`, so a framework host that records
  input through its own store uses the same path. See
  [Execution Backends](framework/execution-backends.md).
* **LLM edge cases are measured before they are fixed.** Drivers report the
  provider's raw stop reason (Bedrock now normalizes `max_tokens` to `length`)
  and count tool calls discarded or run from a truncated response;
  `llm.generation`, `turn.completed`, OTel, Braintrust and Prometheus carry
  them, and overflow classifier gaps and retry exhaustion are logged. See
  [Observability Providers](operations/observability.md#llm-edge-case-telemetry).

* **AG-UI streams the todo list as shared state.** The shared projector sends
  the agent's `write_todos` list as `STATE_SNAPSHOT`, then `STATE_DELTA`
  patches, and opens a run on a session that has one with its snapshot.
  Server endpoints opt in with `state_visible` (default off, off for Public
  Chat); client `state` is still not read. See
  [AG-UI Channel](integrations/ag-ui.md#shared-state).
* **One store trait for turns.** durable-engine's runner store
  (`DurableStoreBackend`) and task store (`TaskStore`) merged into
  `TurnStore`, which every `everruns-durable` store gets by a blanket impl and
  the worker implements for its gRPC client; the `DurableExecution` newtype is
  gone and turn steps checkpoint `TurnExecution` directly. See
  [Execution Backends](framework/execution-backends.md).

* **Durable tasks have named queues.** `ActivityOptions::queue` (the
  `durable_task_queue.queue` column, server migration 178) names the queue a
  task goes to, and a claim takes one queue's tasks only. A PostgreSQL
  `DurableBackend` routes its sessions' steps through a queue of its own
  instead of tagging their activity type. See
  [Execution Backends](framework/execution-backends.md).

## 2026-10-05

* **The generic workflow engine is an opt-out feature.** `everruns-durable`'s
  `Workflow`/`Activity` traits, `WorkflowExecutor`, `WorkerPool` and
  `TimeoutManager` sit behind the experimental `workflows` feature, on by
  default for crates.io users and off for every workspace crate, none of which
  runs it. `examples/order_pipeline.rs` is its isolated consumer and runs in
  CI. See [Durable Execution Engine](operations/durable-execution-engine.md#workflow-engine-feature).

* **Maintenance sweeps run once per cluster.** Blob GC, event and Sandbox
  history retention and the Memory and Knowledge index source syncs moved from per-replica
  interval loops to durable `@every` schedules served by a server-side job
  pool. See [Scheduled Tasks](operations/scheduled-tasks.md#cluster-once-maintenance-jobs).

* **Stale-task reaping and schedule bootstrap moved into durable.** The
  reclaim loop and dead/sealed-task terminalization the server ran inline
  are now `everruns_durable::maintenance`, with a `ReapHandler` for the turn
  lifecycle, and system schedules are declared once through
  `ensure_schedule`, which also accepts fixed `@every` periods. See
  [Durable Execution Engine](operations/durable-execution-engine.md#stale-task-reaping)
  and [Scheduled Tasks](operations/scheduled-tasks.md#system-schedules).

* **Modal sandboxes, in a new everruns-integrations crate.** The `modal`
  capability runs agent code in Modal VM sandboxes (own kernel) or gVisor
  containers over Modal's gRPC API, with files, snapshots and tunnels.
  It is the first module of `everruns-integrations`, which folds vendor
  integrations behind per-vendor features like `everruns-drivers`.
  Experimental (dev grade only). See [Modal Sandboxes](integrations/modal.md).

* **Durability concepts point at the durable-engine crate.** Turn-driver
  ownership (`DurableExecution`, `TurnTaskDriver`, turn conventions) is
  recorded in `everruns-durable-engine` rather than the worker, the crate is
  recorded as published, the durable schema's hash-marker skip and `connect`
  are noted, and the crate dependency graph shows durable-engine between the
  worker or facade and the generic engine. See
  [Durable Execution Engine](operations/durable-execution-engine.md) and
  [Execution Backends](framework/execution-backends.md).

* **Durable PostgreSQL tests run in CI and cannot skip there.** The durable
  shard now runs durable-engine's PostgreSQL backend tests and the facade's
  backend conformance suite against its database, with
  `EVERRUNS_REQUIRE_POSTGRES_TESTS` turning a missing `DATABASE_URL` into a
  failure. Before, both skipped in every CI job. See
  [Execution Backends](framework/execution-backends.md#success-bars).

* **Turns use no circuit breaker.** Corrected the provider rate-limiting
  dismissal, which claimed the durable worker wrapped reason with
  `DistributedCircuitBreaker`. Breakers exist in `everruns-durable` and the
  server exposes them over gRPC and the admin API, but no turn step uses
  them; provider failures are absorbed only by per-call retry in the drivers.
  The worker's unused breaker client methods were removed. See
  [Dismissed Options](project/dismissed-options.md#process-level-llm-provider-rate-limiting-eve-7).

* **Proposed: one entity actions menu.** Every entity page gets one header
  overflow menu with fixed groups (entity actions, Record, Lifecycle) for
  secondary functions such as History and Manager notes. See
  [Entity Actions Menu](ui/entity-actions-menu.md).

* **Proposed: entity history replaces agent versions.** Every change stores a
  secret-free snapshot (secrets as keyed fingerprints), any point can be
  restored, and public versions, semver and pinning are retired. See
  [Change Reasons and Manager Context](execution/change-reasons-and-manager-context.md).

* **The durable framework backend runs on PostgreSQL.**
  `durable::Backend::postgres(store)` runs facade turns on a shared
  `everruns-durable` PostgreSQL store. Each backend instance claims only the
  steps it routed to itself (its own key in the activity type, no schema
  change), since a step needs a runtime only the attaching engine has; a
  session attached again ends the workflow a gone process left running for it,
  and an interrupted turn continues from the session log as on every backend.
  The conformance suite passes on it with `DATABASE_URL` set. See
  [Execution Backends](framework/execution-backends.md#postgresql).

* **Durable turn tickets wake on completion.** A `DurableRunner` ticket over
  the memory store waits on a workflow-end signal the store fires at every
  terminal status write, with a 1 s poll as the fallback, instead of a 50 ms
  poll: a facade durable turn's single-session p50 drops from ~52 ms to
  2.3 ms text and 5.7 ms tool (in process: 2.0 and 5.0). PostgreSQL tickets
  still poll. See
  [Execution Backends](framework/execution-backends.md#benchmark).

* **Turn backend benchmark.** `crates/everruns/benches/turn_backends.rs`
  measures per-turn latency and throughput of the in-process and the durable
  memory backend with llmsim at zero model latency; the facade CI job runs its
  smoke. The 50 ms ticket poll is the durable backend's single-session
  latency (~52 ms p50 against ~2 ms in process; ~1.7 ms of real overhead with
  the poll at 1 ms). See
  [Execution Backends](framework/execution-backends.md#benchmark).

* **Lua code-mode calls pass the target tool's policy.** `tools.<name>(...)`
  runs the act phase's pre-tool chain and the target's schema as the nested
  tool before running it, and the post-tool hooks on its result, and refuses
  without the turn's policy. See [Lua Execution](execution/lua-execution.md) and
  [Threat Model](security/threat-model.md) TM-LUA-009.

* **Proposed: change reasons and manager context.** Every mutating command
  takes a `reason` from any surface, recorded by `Command::run` in a generic
  entity history; every managed entity can carry manager-only notes its own
  runtime never sees. Design, contracts and test plan in
  [Change Reasons and Manager Context](execution/change-reasons-and-manager-context.md).

* **The durable backend continues parked and interrupted turns.** Facade
  sessions on `durable::Backend` now resume turns parked on client-side tool
  calls and turns a process exit cut off mid-act, with in-process semantics
  and results, and see parked calls as in process. The cross-backend
  conformance suite (`crates/everruns/tests/backend_conformance/`) replaces
  the parity tests and passes on both backends; `TurnBackend` still has no
  `recover()`. See [Execution Backends](framework/execution-backends.md).

* **Facade sessions can run on the durable backend.** The `everruns`
  `durable` feature (opt-in, experimental) adds `durable::Backend`, selected on
  the engine builder: turn steps run as queued, checkpointed tasks on an
  in-memory store, driven by in-process workers over the session's runtime,
  with in-process steering and cancellation semantics. PostgreSQL waits on
  session routing across processes. See
  [Execution Backends](framework/execution-backends.md).

* **Integrations ship against contracts alone.** The runtime SPI (capability,
  tool, tool context, session, message, event) moved from core into
  `everruns_contracts::runtime` behind a `runtime` feature; core re-exports each
  module at its old path. The provider isolation guard now rejects a normal or
  build edge from any integration to core. See [Crate Layout](project/crate-layout.md).

* **The durable backend is a published crate.** `everruns-durable-engine`
  (`DurableRunner` as an experimental `TurnBackend`, `TurnTaskDriver`, the
  in-memory and Postgres-direct stores) joins the crates.io publish set; the
  worker keeps the gRPC transport. See
  [Crate Layout](project/crate-layout.md) and
  [Execution Backends](framework/execution-backends.md).

* **Turns run through one execution backend seam.** Core's host defines the
  experimental `TurnBackend`; the facade session actor runs every turn through
  its in-process default with no behavior change. `everruns-durable-engine` is
  planned as the published durable backend, keeping queue plus per-step
  checkpoint rather than `Workflow` replay. See
  [Execution Backends](framework/execution-backends.md),
  [Crate Layout](project/crate-layout.md) and
  [Dismissed Options](project/dismissed-options.md).

## 2026-10-04

* **Sandbox Template is the hosted configuration resource.** A Sandbox Template
  has immutable revisions; an Agent version owns `sandbox_policy`; a Session
  pins one resolved primary Sandbox that can replace physical provider compute.
  Framework `Environment` remains an internal execution context, not a hosted
  product resource. Legacy Environment routes, keys, and ID prefixes are
  input-only compatibility aliases. See [Sandbox Platform Architecture](harnesses/sandbox-abstraction.md)
  and [Sandbox Templates](harnesses/sandbox-templates.md).

* **Decision models share provider authentication.** Direct TypeSafe and OpenRouter use the same neutral System One contract. Tenant models retain service/profile identity, support an explicit decision default, and bind Jev through host credential, egress, budget and usage boundaries. Utility guardrails remain deployment-owned. See [Decision Service](operations/decisions-service.md).

* **Sandbox platform proposal separates configuration, primary execution, and
  fleets.** Sandbox Template is reusable versioned configuration; a Session owns at
  most one recoverable primary Sandbox over a durable Workspace; agent-managed
  Sandboxes are explicit resources. Harnesses may seal execution, with Bashkit
  Worker as the fixed Bashkit child of Worker. See
  [Sandbox Platform Architecture](harnesses/sandbox-abstraction.md).

* **Browserless browsers have no network of their own.** Pages run in a context
  with a dead proxy and Everruns performs every request they make, so redirects
  and DNS answers to private, loopback, link-local, or metadata addresses are
  refused per hop. Stateless tools use one-shot CDP browsers; REST is only a
  fallback for Browserless cloud tokens without CDP. See
  [Threat Model](security/threat-model.md) TM-TOOL-056.

* **Background runs pass the target tool's policy.** `spawn_background` puts its
  target call through the act phase's pre-tool chain and the target's schema
  before scheduling or starting it, runs only the authorized arguments, and runs
  the post-tool hooks on the result before it is persisted or signalled. See
  [Tool Execution](execution/tool-execution.md) and
  [Threat Model](security/threat-model.md) TM-TOOL-055.

* **MCP server card version is the platform release.** The card's `version` and
  `serverInfo.version`, and the server info on `initialize` and `server/discover`,
  are the running package version. A release tag updates them; they are not a
  separately edited string. See [MCP](integrations/mcp.md).

* **Client-side tools pass server pre-tool policy.** Approval, guardrail, and
  user hooks run before a client execution request, including mixed batches. A
  denial is a tool result and is absent from the request; an allowed call keeps
  the hook's arguments. See [Client-Side Tools](execution/client-side-tools.md)
  and [Threat Model](security/threat-model.md) TM-CLIENT-005.

* **Browserless tools honor the session network ACL.** Screenshot, content, scrape,
  interact, navigate, and persistent browsers reject hosts the session excludes,
  including redirects and discovered requests on transports that can pause them.
  See [Threat Model](security/threat-model.md) TM-TOOL-053.

* **Portable agents share one contract.** File/folder/ZIP loading, legacy Markdown,
  validation and semantic diffs reuse one codec across Platform and Framework
  hosts. See [Portable Agent Packages](runtime-resources/agent-packages.md).

* **Playground list is dense rows.** The library groups the current page by day,
  agent, or none, filters by agent, and opens a chat from the row. Agent and
  virtual-user chips still link to their pages. See
  [Information Architecture](ui/information-architecture.md).

## 2026-10-03

* **Agent testing belongs to Playground.** The Agent masthead opens Playground setup with the
  Agent preselected; personal Chats remain bound to the managed Platform Chat Agent and its fixed
  runtime. Playground owns Agent, harness, virtual-user, and Environment selection. See
  [Agent Page](ui/agent-page.md) and [Sandbox Templates](harnesses/sandbox-templates.md).

* **Hosted feature enrolment names and defaults.** Internal features require
  platform enrolment; adoption remains organisation opt-in. The seven features
  already offered for SaaS opt-in now use adoption defaults, preserving that
  behavior without enabling every organisation. See [Feature Flags](security/feature-flags.md).

* **Paid CI coverage follows provider/model changes and a nightly sweep.**
  Ordinary Rust merges retain llmsim workflows without provider credentials.
  Scheduled and manual live coverage still runs only trusted main-branch code.
  See [Threat Model](security/threat-model.md) TM-CI-001 through TM-CI-003.

* **Actionable health issues.** Add persistent Slack permission and credential
  issues, existing notification delivery, contextual warnings, guided reconnect
  and verified recovery for existing installations. See
  [Actionable Health Issues](operations/health-issues.md).

* **Agent page tabs stay in the address.** Switching Agent, Preview,
  Integrations, Stats, or Sessions writes `?tab=` (the Agent tab omits it), so
  a refresh or a shared link reopens the same tab. See
  [Agent Page](ui/agent-page.md).

* **Feature rollout grades.** Per-feature grades now own deployment availability,
  organisation defaults, and tenant versus platform configuration authority.
  Environment overrides select grades; explicit false organisation overrides
  preserve production opt-outs. See [Feature Flags](security/feature-flags.md).

* **Personal ChatGPT plan drivers.** Shared OAuth, Responses transport, and
  legacy Codex drivers live in `everruns-drivers`; host adapters supply storage
  and browser navigation. Self-hosted account connections require deployment
  enablement plus org opt-in and exact personal runtime ownership. See
  [Providers](foundations/providers.md#personal-chatgpt-plan-connections).

* **Session storage reads require session view.** Listing key/value entries
  or secret names now evaluates `SESSION_VIEW` before the store is read, so a
  same-org caller a custom resolver denies cannot see plaintext values or
  secret names (EVE-1180). Writes stay on `SESSION_MANAGE`. See
  [Threat Model](security/threat-model.md) TM-AUTHZ-023.

* **Image provider egress.** `gpt_image_gen` sends generation and edit
  requests through the host egress boundary with DNS pinning, the session
  network ACL, and no redirects, so an org-configured base URL cannot reach
  internal addresses or carry the provider key to another origin (EVE-1174).
  See [Capabilities](execution/capabilities.md) and TM-LLM-047.
* **Untrusted host shell no longer trusts a program name.** `rg --pre` and the
  other options that start a program ask for approval, including when
  containment is already `danger-full-access`. `git status` asks unless the
  command disables repository fsmonitor and hooks. See
  [Threat Model](security/threat-model.md) TM-BASH-028.

* **A2A outbound SSRF hardening.** External A2A delegation DNS-pins discovery
  and every AgentCard interface URL, disables redirects, keeps the merged
  network ACL, and rejects `allow_local_urls` outside `DEPLOYMENT_GRADE=dev`
  (EVE-1173). See [A2A Capability](integrations/a2a-capability.md) and
  TM-AGENT-024.

* **Anonymous PAT mode transition closed.** Leaving `AUTH_MODE=none` revokes
  PATs owned by the seeded anonymous admin, and PAT validation rejects that
  identity even if a stale row remains (EVE-1153). See
  [Authentication](security/authentication.md) and TM-AUTH-032.

* **Sandbox secret forgery closed.** Capability-owned sandbox secret prefixes
  (`container_sandbox:`, `daytona_sandbox:`, `e2b_sandbox:`, `deno_sandbox:`,
  `sprites_sprite:`) are reserved from user-facing `secret_store`, and
  container-sandbox tools re-inspect Docker `managed-by`/`session` labels
  before every op that uses a stored ID (EVE-1151). See
  [Container Sandbox](runtime-resources/container-sandbox.md) and
  TM-SANDBOX-004.

* **Time-annotation echo cleanup.** `message_metadata` also strips echoed
  `<facts>` blocks and whole-message degenerate `time <junk>` lines from
  assistant text, and ReasonAtom applies the same filters to `reason.item`
  summaries so the work log does not show annotation junk. See
  [Capabilities](execution/capabilities.md#messagemetadata).

* **Crate layout target.** Published crates go from 52 to about 37 in seven
  release-sized steps: `everruns-contracts` absorbs provider, capability, and
  model profiles plus platform's extension traits; `everruns-platform` splits
  into `everruns-capabilities` and server-owned records; engine, host,
  builtins, MCP, and AG-UI fold into core behind features; the worker gets a
  private `everruns-durable-engine`. Only the server and durable open a
  database. See [Crate Layout](project/crate-layout.md).

## 2026-10-02

* **Platform Chat replaces its preview and legacy implementation.** The shell,
  docs, and durable-memory composition is the canonical chat surface for every
  org, without an opt-in flag. Reconciliation preserves canonical IDs, moves
  preview bindings, and retires the preview; resumed chats restore docs and
  shared memory. See [Platform Chat](harnesses/platform-chat.md).

* **Seeded Agents API sessions.** A turn that creates an OpenAI Agents API
  provider session after one was replaced, released, or lost sends a bounded
  transcript of the earlier turns (user and assistant text, completed tool
  call/result pairs; newest 200 entries, 32 KiB) as a fenced user message ahead
  of its input, since create `input` takes user-role messages only (verified
  live). See [OpenAI Agents API Runtime](execution/openai-agents-api-runtime.md#portability);
  TM-LLM-046.

* **Agents API live conformance passed.** With the OpenAI organization funded,
  `live_conformance_one_client_function_and_one_allowed_mcp_tool` completed a
  turn with one client function and one allowed MCP tool (EVE-1144). See
  [OpenAI Agents API Runtime](execution/openai-agents-api-runtime.md#live-validation).

* **Memory model moves into knowledge.** The design note on how the
  per-session Workspace and the durable org Memory tiers relate left the public docs Advanced group
  and is now [Memory Model](runtime-resources/memory-model.md). The user-facing
  part stays in the docs Memory Scopes page.

* **Parked approvals and questions survive a restart.** A turn waiting on a
  person blocks inside its act, so a killed process leaves it in the log
  without an end. `Session::interrupted_turn` reports such a turn from the
  log, and `Session::resume_interrupted_turn` runs its unfinished calls again
  in the same turn, which asks again under the same tool call ids. serve
  resumes it when a session comes back, only when every unfinished call waits
  on a person, so an ungated tool is never re-run. See
  [serve](framework/serve.md) and [AG-UI Channel](integrations/ag-ui.md).

* **`everruns-durable` is a generic engine.** It no longer depends on any
  `everruns-*` crate or knows about agents and turns. `DurableExecution` and
  the turn conventions (`user_message` signal, idempotent waiting-turn
  resolution tasks) moved to the worker; the store's new-run claim is
  `EventLog::try_start_new_run`; dedupe is the generic
  `ActivityOptions::dedupe_by_activity_id`. Wire strings, SQL effects and task
  ids are unchanged. See
  [Durable Execution Engine](operations/durable-execution-engine.md).
* **`everruns-durable` joins the crates.io publish set.** The crate ships its
  own idempotent PostgreSQL schema, applied by
  `PostgresWorkflowEventStore::migrate`, so it no longer depends on the server
  migrations to be usable; a CI drift test keeps the two identical for the
  durable tables. Bench support moved behind a `bench` feature. See
  [Durable Execution Engine](operations/durable-execution-engine.md#persistence).
* **Ready-made AG-UI route.** The facade's `ag-ui-axum` feature adds
  `everruns::ag_ui::AgUiHandler`: a required pluggable authorizer
  (`StaticToken`, `Unauthenticated`, closures), thread resolution through
  `AgUiThreads` scoped to the caller, and SSE with the server's framing and
  15-second keepalive; serve frames its route with the shared
  `sse_response`. See [AG-UI Channel](integrations/ag-ui.md#framework) and
  TM-AUTH-030.
* **AG-UI threads in the framework.** `everruns::ag_ui::AgUiThreads` maps
  `threadId` to a session through a pluggable `ThreadStore` (in-memory, or
  SQLite behind `local`), so a host keeps a thread across restarts, and a
  thread's first run seeds the client's earlier user and assistant messages
  as history (`AgUiOptions::seed_history`, also used by serve). See
  [AG-UI Channel](integrations/ag-ui.md#framework); TM-TENANT-017 and
  TM-DOS-045.
* **Opt-in Responses WebSocket transport.** The OpenAI driver can stream a
  call over OpenAI's Responses WebSocket mode (`openai/websocket` driver
  option, or `OpenAIChatDriver::with_websocket_transport`) on `api.openai.com`,
  reusing one socket across the turns of a tool loop and falling back to SSE
  when the socket fails before the first event. SSE stays the default. The wire
  contract, including what was inferred from the official SDK, is pinned in
  [OpenAI Responses WebSocket Transport](foundations/openai-responses-websocket.md);
  threat-model entry TM-LLM-045.
* **PR Reviewer and Security Scanner templates.** Two agent examples carry a
  guided setup (connect the agent's GitHub App, pick a repository, choose
  settings, create the trigger) on the existing import path. The `github`
  capability gains `submit_github_pull_request_review` (inline comments,
  cannot approve, deterministic repeat suppression), `upsert_github_issue`
  (fingerprint-keyed findings) and the opt-in `create_github_pull_request`,
  with `allow_pull_requests` and `private_issues_only` settings the tools
  enforce. See [GitHub review and security agent
  templates](integrations/github-agent-templates.md) and TM-GHAPP-008 to 011.

## 2026-10-01

* **Everruns agents can delegate to external AG-UI agents.** The
  `ag_ui_delegation` capability (behind `everruns-platform`'s `ag-ui` feature,
  on in the product build) adds `spawn_agent` target `external_ag_ui` for agents
  listed in its config. Each delegation is an `external_ag_ui` session task: a
  remote interrupt parks it in `awaiting_input`, `message_task` answers with a
  resuming run, `cancel_task` closes the stream, and a stream lost with its
  worker fails as orphaned. Recorded in
  [AG-UI Capability](integrations/ag-ui-capability.md), threat-model entries
  TM-AGENT-030 to TM-AGENT-032.
* **Everruns can consume AG-UI streams.** `everruns-ag-ui` gained a consumer
  pipeline (1.0 processing model, sequencing rules, chunk expansion, result
  assembly, the resume coverage rule) and an HTTP/SSE client behind the
  `client` feature, held to upstream's client conformance corpus. Recorded in
  [AG-UI Channel](integrations/ag-ui.md#consumer-rules).
* **AG-UI subagents, run metadata and capabilities.** With `subagents_visible`
  (default off) subagent tasks stream as `SUBAGENT_*` keyed by task id, their
  posted text and summary attributed by `subagentRunId`; segments still open
  when a run ends close as `suspended`. Run events carry `metadata.everruns`
  (turn, model with usage, session id for identified callers), and
  `GET /v1/e/{endpoint_id}/ag-ui/capabilities` serves a 1.0
  `AgentCapabilities` derived from the endpoint config. Recorded in
  [AG-UI Channel](integrations/ag-ui.md#subagents) and threat-model entry
  TM-API-026.
* **serve speaks AG-UI.** `serve`'s `ag-ui` feature mounts
  `POST /v1/e/{agent}/ag-ui`, the server's channel route shape, as a thin layer
  over `Session::ag_ui_with`. The facade gained `InterruptSource`, so serve's
  existing parked approvals and questions become interrupts and either API
  answers them; `examples/serve/ag-ui` drives it with `@ag-ui/client`. See
  [AG-UI Channel](integrations/ag-ui.md#serve).
* **The framework serves AG-UI.** The `everruns` facade's `ag-ui` feature
  answers an AG-UI 1.0 request from any session with the shared projection,
  and an in-memory `InterruptGate` turns in-process `ask_user` and approval
  waits into interrupts that `RunAgentInput.resume` answers. See
  [AG-UI Channel](integrations/ag-ui.md#framework).
* **AG-UI frontend tools.** `RunAgentInput.tools` become the session's
  client-side tools; a parked call to one streams to the consumer and ends the
  run in success with `pendingToolCallIds`, and the next run's trailing `tool`
  messages resume the turn. Recorded in
  [AG-UI Channel](integrations/ag-ui.md#frontend-tools), threat-model entry
  TM-DOS-044 and the updated TM-LLM-020.
* **Agents can wake on MCP events (EVE-1121, inbound half).** An `mcp_event`
  trigger subscribes, through MCP Events, to an event on one of the agent's own
  MCP servers with that attachment's credential, and keeps the subscription in
  step with the trigger: a fresh encrypted secret on create, enable and change,
  a periodic refresh before `refreshBefore`, unsubscribe on disable and delete.
  Signed deliveries to `/v1/e/{ingress_id}/mcp-events` feed the shared trigger
  event pipeline; anything not live answers `410`. Behind `mcp_events`.
  TM-TRIGGER-005. See [MCP Events](integrations/mcp-events.md#inbound-mcp-event-triggers).
* **Computer use phase 2 (EVE-1133).** [Computer use](execution/computer-use.md)
  now swaps the `computer` function tool for OpenAI's native `computer` tool
  and Anthropic's `computer_toolset_20260801` on models that have them,
  through a provider-neutral driver option, with execution unchanged. Calls
  that type, press Enter, navigate, or carry provider safety checks pass a
  hard per-call approval gate in hosted sessions, built on the
  [tool approval](execution/tool-approval.md) gate; one answer carries a call
  through both gates. The session UI shows result screenshots as thumbnails.
* **AG-UI runs interrupt and resume.** A turn parked on `ask_user` or a tool
  approval ends its AG-UI run with the 1.0 interrupt outcome, and
  `RunAgentInput.resume` answers it through the shared resolvers. Approvals
  are answerable by the client only on endpoints that opt in, and token usage
  is reported only when the endpoint enables it. Recorded in
  [AG-UI Channel](integrations/ag-ui.md#interrupts-and-resume), threat-model
  entries TM-TENANT-016 and TM-TOOL-052.
* **The OpenAI Agents API backend enforces Everruns policy at its tool and
  output boundaries (EVE-1124).** A call the tool pipeline parks (an approval,
  a client-side tool, a connection setup) parks the Everruns turn while the
  provider's required action stays open; only a recorded approval runs it
  again, and a rejection or expiry submits a failure. Budgets stop the turn
  before more tools run, output guardrails judge every remote message, and
  configurations with provider-run tools or MCP credentials are refused.
  TM-LLM-043 is mitigated. See [OpenAI Agents API Runtime
  Backend](execution/openai-agents-api-runtime.md#policy-at-the-tool-and-output-boundaries).
* **The OpenAI Agents API backend is durable and opt-in (EVE-1123).** A session
  selects it with the `openai_agents_api_runtime` capability, which the
  `openai_agents_api` org flag gates. One encrypted, lease-fenced checkpoint per
  session holds the provider session id, stream cursor, item correlations, and
  input and tool-result outboxes, written ahead of every provider call and local
  effect. Recovery adopts an uncertain create by session metadata (creates are
  not idempotent at OpenAI), retries input under its idempotency key (which
  OpenAI honors), and reconciles from saved items after a disconnect. Tools run
  only as client functions through the Act pipeline. Threat-model entries
  TM-TOOL-051 and TM-LLM-043. See [OpenAI Agents API Runtime
  Backend](execution/openai-agents-api-runtime.md).
* **Hosted sessions have a hard tool-approval gate (EVE-1140).** [Tool Approval](execution/tool-approval.md)
  records how `tool_approval` works where a turn cannot block on a human: the
  gate defers an undecided call, the turn parks on an `approve_tool_call`
  request, a person answers through `POST /v1/sessions/{id}/tool-approvals`, and
  the retried call finds the decision in session storage on whichever worker
  runs it. One-off approvals bind to the exact arguments; unanswered requests
  expire as not approved. TM-TOOL-008 is mitigated.
* **AG-UI channel moved to 1.0.** The endpoint now uses the in-repo
  `everruns-ag-ui` types instead of the pre-1.0 community crate, streams
  `REASONING_*`, reports cancelled runs as a `cancelled` outcome, answers the
  `protocolVersion` handshake, and closes every open message before the
  terminal event. The projection moved into the crate so framework and `serve`
  can share it. Recorded in [AG-UI Channel](integrations/ag-ui.md).

## 2026-09-30

* **Virtual users now own runtime profiles, preferences, and connections.** [Virtual Users and Everruns Users](runtime-resources/virtual-users.md) separates organization-scoped agent consumers and service accounts from management users. Chats and personal connection settings use the console default runtime account. Per-input authority replaces historical-owner credential resolution; ambiguous global grants require an explicit destination.

* **Computer use**: Added the provider-neutral [computer use](execution/computer-use.md)
  contract and its first backend on Browserless, with threat-model entries
  TM-TOOL-048 to TM-TOOL-050.

* **Decisions are now answered by pluggable decision drivers (EVE-1117).** The
  service was hard-wired to TypeSafe; OpenAI's Decisions API made "there will
  be other classifiers" concrete. A `DecisionDriver` declares capabilities and
  calibration, a host registry routes by model id (`driver/model`, `jev-*`,
  else the default), and the router is the `DecisionsService` callers already
  hold. The first vendor-free driver, `llm`, answers through the utility model
  and reports `calibrated: false` rather than inventing probabilities, which
  needed a new `DecisionOutcome::calibrated` flag. `DECISIONS_DRIVER` and
  `DECISIONS_MODEL` select the default; a TypeSafe-only deployment behaves as
  before. Recorded in [Decisions Service](operations/decisions-service.md#decision-drivers).
* **OpenAI's Decisions API is a preview decision driver (EVE-1118).** Its wire
  shape is unpublished and our account is not enabled yet, so the driver ships
  opt-in (`DECISIONS_OPENAI_PREVIEW`), unpublished, with the inferred shape
  isolated in one module. It never spreads the API's single confidence into a
  distribution. The Jev comparison waits for access.

* **OpenAI's Agents API can be wrapped only behind a constrained runtime boundary.**
  Function required actions preserve Everruns approvals and guardrails, but direct
  MCP and OpenAI built-ins do not expose an equivalent interception point. The
  recommended first slice keeps the native runtime as default, maps provider
  events into the existing session protocol, and treats Everruns as the product
  ledger while OpenAI owns live loop state. A feature-gated
  prototype covers one function tool, one MCP tool, event projection, and config
  import. It ran end to end against the live API on 2026-09-30 with one
  function and one MCP tool; the recorded stream is the test fixture. See [OpenAI Agents API Runtime Backend](execution/openai-agents-api-runtime.md).
## 2026-09-29

* **Virtual users have a proposed replacement design for the runtime identity
  paths.** [Virtual Users and Everruns Users](runtime-resources/virtual-users.md)
  separates management accounts from agent consumers, generalizes agent identities
  into one runtime aggregate, unifies connection ownership, and makes invocation
  authority explicit. It covers external identity binding, console proxying,
  Platform Chat management authorization, and cutover. Organization scope is
  accepted: virtual users and their connections are org-scoped, with explicit
  destinations for migrating existing multi-org grants. UI and API proposals
  map console self-service, virtual-user management, and verified external
  consumer access onto the same model. No product behavior changes.

## 2026-09-28

* **Inbound MCP form mode elicitation now has a design, and the blocker that
  held it turned out to be sharper than recorded.** EVE-1068 framed the open
  question as attribution: a third-party server authoring a question that renders
  in Everruns' chrome. The deeper reason form mode was left undeclared is in
  `protocol.rs` — a form answer is carried by a tool result, so it lands in the
  event log and permanently in model context, which is what URL mode exists to
  avoid. That forces a conclusion the issue did not anticipate: `ask_user`'s
  `Secret` kind cannot serve this path at all, because its `session:{name}`
  reference is meaningless to a remote server, so the only behaviors available
  are refuse or leak. The design refuses credential-shaped properties and names
  URL mode back to the server. It also specifies the per-server
  `elicitation_policy` that `mcp-servers.md` listed as not built, defaulting to
  URL-mode-only so existing configuration is unchanged. Recorded in [inbound form
  mode elicitation](integrations/mcp-form-elicitation.md), threatened as
  TM-TOOL-043 through TM-TOOL-047. No product code changes.

## 2026-09-24

* **serve now speaks the everruns server's `/v1` API and is a thin layer over
  `everruns::Engine`.** Sessions, messages, `/sse` (`since_id` or
  `after_sequence`), `/events`, cancel and `/question-answers` match the
  server's shapes, so the official Rust SDK drives a serve app (tested
  end to end). The eve-style wire API and serve's own event log are gone: the
  durable log, approvals and `ask_user` are the runtime's, and serve keeps only
  a session catalog. eve stays the ergonomics reference, not the wire shape.
  Recorded in [serve](framework/serve.md).
* **An experimental app framework, serve, pairs Topcoat's API shape with eve's
  hosting model on the existing runtime.** Attribute macros register agents,
  tools, channels, schedules, connections and evals at link time. The built
  binary emits a manifest that is the whole contract with a host. One `/v1`
  wire API resumes by event-log cursor instead of a continuation token. Because
  the binary is the behavior, a restart no longer needs the application to
  reattach agents by hand. It is a proof of concept, published as
  `everruns-serve` (plus `-macros` and `-build`, since crates.io `serve` is
  taken) but outside the stability policy. Recorded as
  [serve](framework/serve.md).
* **Anthropic Infinity Context now has an append-only design target instead of
  a notice-only cache fix.** Stabilizing `[N earlier messages ...]` would still
  move the recent window, so it cannot preserve the full prompt prefix. The
  selected design keeps the prior wire-level `messages` array unchanged on
  explicitly supported direct Anthropic models, uses threshold server-side
  compaction to reduce the model-visible context, and uses automatic top-level
  prompt caching so cache markers do not edit prior message blocks. Raw history
  remains lossless and checkpoints store only an encrypted replay optimization;
  unsupported or rewrite-unsafe configurations retain the legacy window.
  Recorded in [Anthropic Infinity Context
  Compaction](runtime-resources/anthropic-infinity-context-compaction.md).
* **Framework observability has a design: listeners on the Engine, OTel and
  Braintrust as opt-in values.** The `everruns` crate could only be observed by
  pulling `Session::events()` per session, and the existing OTel and Braintrust
  exporters were wired only by the server. The proposal registers push listeners
  once on `Engine::builder()`, so new, resumed and spawned sessions are all
  covered; app listeners see the reviewed `SessionEvent` while the built-in
  exporters get the lossless core event internally. Each listener drains its own
  bounded queue in commit order, so an observer can drop events but never slow a
  turn, and `Engine::shutdown` flushes what short-lived programs would otherwise
  lose. The framework never installs global tracing state. Tracked as EVE-1100
  and EVE-1101. See [Framework Event Listeners and
  Observability](framework/observability.md).

## 2026-09-19

* **A live PoC settled how far Slack one-click install can go, and disproved two
  assumptions on the way.** `apps.manifest.create` accepts the manifest we
  already generate — whole, `agent_view` and all — and returns the signing
  secret with it, so three of the four fields an operator types today can be
  obtained without a human. It does not verify `request_url` at save time. It
  cannot, however, install without a consent screen. So the target is one
  consent per agent, not zero, and a Slack Marketplace listing turns out not to
  be on the critical path at all. Disproved: that one-click and per-agent bot
  identity were in tension (they are not), and that the app cannot exist before
  the endpoint is live (the API path does not check). Also surfaced a real
  defect — the generated manifest declared no `oauth_config.redirect_urls`, so
  no generated app could ever be OAuth-installed; invisible until now because
  the copy-paste flow never runs OAuth. Recorded as [Slack One-Click
  Install](integrations/slack-one-click-install.md).

## 2026-09-19

* **The utility LLM picks its backend from which key you set, and the model is
  now an env var.** `UTILITY_OPENROUTER_API_KEY` routes internal model work
  (Analyze, Health, `llm_judge` guardrails) through OpenRouter,
  `UTILITY_OPENAI_API_KEY` keeps calling OpenAI directly, and
  `UTILITY_LLM_MODEL` overrides the model on whichever backend was selected. A
  separate `UTILITY_LLM_PROVIDER` variable would have been a second thing to
  keep in sync with the secret that has to be rotated anyway, so presence of
  the key *is* the selection; OpenRouter wins when both are set, with a startup
  warning, because the key an operator just added is the deliberate one.
  Defaults stay one model named two ways: `gpt-5.6-luna` on OpenAI,
  `openai/gpt-5.6-luna` on OpenRouter. The deployment-owned contract is
  unchanged — `UtilityLlmRequest` still carries no model and no credential, so
  the new knob is unreachable from agent, session, or API input (TM-LLM-021).
  See [Utility LLM Service](operations/utility-llm.md).

## 2026-09-18

* **Platform Chat v2 measured against v1, and the shell surface holds: 48/63 to
  43/63 on 4.47 tool calls against 5.02.** Three trials per case against
  `meta/muse-spark-1.3-contributor`, same dataset, same in-process control
  plane, same model, differing only in the harness the offline eval subject
  reproduces. Only one case moved for a reason rather than by noise:
  `cli-tree-help-instead-of-guessing` went 0/3 to 3/3, because on v1 the model
  has `discover` and reaches for it, while on v2 `--help` is the only route and
  the prompt says so. Two percentage points over three trials is not a win, so
  the reading is "at least as good, on fewer calls", and the two shared failures
  (`agents-find-by-purpose`, `plugin-agent-connection-preflight`) find the right
  commands on both arms and then exceed their tool-call budget, which is the
  model's problem and not the surface's. See
  [Platform Chat v2](harnesses/platform-chat.md).

* **v2's prompt was instructing the model to use tools v2 had already given up.**
  Moving `platform` to the shell surface stopped it contributing `discover`,
  `query` and `execute`, but the harness prompt still said to pass loops to
  `execute` and to verify with `query`. A prompt naming an absent tool is worse
  than silence: the model spends a turn discovering the gap. The passages are
  shell-native now, and a test holds that v2's prompt names no tool it does not
  ship, which is the kind of drift only a test catches, since neither half is
  wrong on its own.

* **The eval now runs the model's shell script instead of splitting it by hand.**
  The offline subject's v1 path approximated bash: statements were split on
  newlines and `;`, so `for … do … done` never ran as a loop, and pipelines were
  truncated at the first `|`. The v2 arm hands the script to a real bashkit
  interpreter with `everruns` as a builtin, so a failure there is the model's or
  the contract's rather than the splitter's. `--help` is the shipped rendering
  on both arms too: it travels as a generated artifact from the real `CliTree`,
  descriptions included, because the hand-rolled version listed bare nouns and
  in v2 `--help` is nearly the whole discovery story.

* **One dataset grades two surfaces by reading a role, not a tool name.** The
  cases name v1's `query` and `execute`; v2 has one `bash` for both. Rather than
  fork the dataset, the scorers classify a `bash` call by what its script runs:
  `execute` when it invokes an operation the catalog marks as a mutation,
  `query` otherwise. Help probes are excluded, because `everruns agents create
  --help` names a mutation and performs none, and a classifier that cannot tell
  those apart fails a read-only case for the behaviour the CLI cases reward:
  reading the help before guessing a flag.

## 2026-09-17

* **Framework APIs gain stability tiers: LLM surface stable, classifier alpha.** Rust's `#[stable]` / `#[unstable]` are nightly-only, so `crates/everruns` marks stability with one-line rustdoc banners defined in the new `crates/everruns/src/stability.rs` helper, recorded in `knowledge/framework/api-stability.md`. First pass marks `llm` stable and `classifier` alpha; unmarked items stay provisional (treat as alpha).

* **Two crates for TypeSafe became one, by removing the constraint instead of
  working around it.** The vendor client sat in `crates/drivers/typesafe` only
  because `everruns-host` held the classifier, and a host dependency
  cannot point at an integration crate without closing the loop
  (integration → platform → host). Moving the service into
  `integrations/typesafe` inverts that: `crates/server` and `crates/worker`
  already depend on integrations, so they compose it into `HostComposition`
  from above. Host no longer knows TypeSafe exists, and the client has one
  home.

* **Classifications got the same promoted surface direct model calls got.** `Classifier`
  and `Classifier::about` mirror `Model::complete` and `Model::completion` over the
  `ClassifierService` contract core already owned — state plus typed questions in,
  calibrated numbers out, and the threshold that decides an outcome staying in
  the caller's code. No streaming, because a classification is one round trip. The
  facade names no vendor: `Classifier::new` takes any service the way `Model::new`
  takes any provider, and `Classifier::simulated` keeps tests offline.
* **A model id was promoted without the catalog behind it.** Applications could
  select a model and call it through the facade, but not ask a provider which
  models it serves: discovery, the profile registry, and driver-kind identity
  sat in `everruns-provider`, so any model picker took a second crate and
  provider-owned types. `everruns::models` promotes listing and metadata on the
  same value-first terms as direct model calls, and a `Provider` now carries the
  driver kind it speaks so profiles resolve for a provider keyed by an
  application name. See
  [Framework Application API Boundaries](framework/application-api.md).

* **A default a caller never sees is worse than an argument they must write.**
  `Classifier::new` took only a service, letting the service's own `jev-latest`
  stand in when no model was named — which read as convenience but meant a
  calibrated threshold could move under a caller who never chose a version.
  It now takes the model up front, exactly as `Model::new` does, so the pair
  reads as one shape: the service is transport, the model is what answers.
  `ClassificationRequest::model` stays optional, because the platform composes
  requests that deliberately name none (THREAT[TM-LLM-037]). See
  [Classifier Service](operations/decisions-service.md).

* **A vendor name can collide with the host language.** The provider type was
  `TypeSafe`, which is the company — but in Rust `TypeSafe` reads as a marker
  about type safety, and `TypeSafeClient` reads as "a type-safe client". The
  vendor never has this problem; we do, because the name landed in a type
  position. `TypeSafeAI` is the company's own full name, cases like the
  neighbouring `OpenAI`, and can only be read as a company. Type names moved;
  the `TYPESAFE_API_KEY` variables, the `typesafe` feature and crate, and the
  stored `typesafe` connection provider did not — they are not type positions,
  and the provider string is persisted. See
  [Classifier Service](operations/decisions-service.md).

* **The agent-facing surface is named for the model, the credential surface for
  the vendor.** The capability is `jev` and its tool is `jev_evaluate`, matching
  the guardrail engine value: an agent author is choosing the thing that
  answers. The connection, the crate, the session secret, and the docs page stay
  TypeSafe, because that is the account the key belongs to. Renamed before
  anything shipped, so no stored config carries the old ids.

* **The guardrail engines were shipped without anyone knowing how well they
  work.** Unit tests proved the plumbing and the fail-open contract; nothing
  measured block rate against false-positive rate, which is the number that
  decides whether a guardrail is usable. The new
  [guardrail-calibration study](../evals/guardrail-calibration/README.md) runs a
  labeled corpus through the shipped decision path across both engines and a
  threshold sweep. Its first run found neither engine dominates at the default:
  `jev` caught every violation but over-blocked a benign staging-table deletion,
  `utility_llm` never over-blocked but missed a path-traversal read. `jev` at
  threshold 70 shed the false positive without losing a violation — evidence
  that the default of 50 is worth revisiting, on a corpus far too small to
  revisit it with.

* **The guardrail engine value now names the model, and the deployment key names
  its role.** The engine value is `jev`, not the service behind it: an agent
  author is choosing between prose a parser has to trust and a calibrated
  number from a specific model, so the config says which model. If the
  classifier is
  ever backed by something else, the value gains a sibling rather than changing
  meaning. The deployment credential became `UTILITY_TYPESAFE_API_KEY`, mirroring
  `UTILITY_OPENAI_API_KEY` — both are platform-owned credentials for internal
  model work, and the name now keeps them visibly distinct from the
  `TYPESAFE_API_KEY` session secret the agent-facing capability falls back to.
  Everything that spends the deployment credential is gated on it: unset means a
  disabled service, jev checks are skipped with a log line naming the variable,
  and the turn proceeds.

* **Model-backed guardrails had a fail-open that read as allow.** `llm_judge`
  and `moderation` prompted the utility model for a JSON verdict and parsed it
  back, so a missing fragment, a parse error, or an unrecognized verdict
  silently downgraded a block to an allow - and moderation simulated a
  probability by asking a text model to write 0-100 per category. Both check
  types now take an `engine`: the new `jev` engine asks a typed question and
  gets a calibrated probability, so the configured `threshold` decides in code
  and there is nothing to misparse. It also answers every check on a stage
  in one request instead of one per check. `utility_llm` stays the default, so
  existing configs are unchanged and the two are directly comparable on the same
  agent. Recorded in [Guardrails](execution/guardrails.md) and the new
  [Classifier Service](operations/decisions-service.md), with the egress and
  steering analysis in TM-LLM-037/038.

* **Operators had no single place to learn what the system model keys do.**
  `UTILITY_OPENAI_API_KEY` was mentioned only in passing on a feature page, and
  the new `TYPESAFE_API_KEY` had nowhere at all. Both are now documented
  together in the public environment-variables reference, including the point
  that a missing key fails open — a guardrail whose engine is unconfigured
  weakens policy silently rather than wedging traffic.

* **Eval coverage gaps are now written down.** Model-backed guardrails have no
  calibration coverage, and typed-judgment tool *use* is unmeasured; both are
  recorded in [Evals](evaluation/evals.md#known-coverage-gaps) with the shape
  each one needs.

* **Typed judgments are now an agent-facing capability too.** The `typesafe`
  integration contributes `jev_evaluate`, so an agent can verify or rate
  something and get numbers back instead of forming a second impression in
  prose. Its user connection is deliberately separate from the deployment key
  that backs the classifier.
* **Integration registration was a linker side effect.** Integration crates
  submitted their capabilities and connectors through `inventory::submit!`, and
  `crates/server/src/lib.rs` and `crates/worker/src/lib.rs` each carried an
  `extern crate` block so the linker kept those crates and their link-section
  submissions. A registry's contents therefore depended on what a binary
  happened to link: dropping a line removed an integration with no compile
  error, the set was maintained in four places (two manifests, two `extern
  crate` blocks), and `everruns-platform`'s own tests saw a different registry
  than production because platform does not depend on the integration crates.
  Each crate now publishes `CAPABILITY_PLUGINS` / `CONNECTOR_PLUGINS` consts and
  the new `crates/integrations-catalog` names every one, with
  `scripts/lib/check-integration-catalog.sh` failing a crate that publishes
  plugins without a catalog entry. Embedders filter or extend `CATALOG`.
  `SessionSandboxProviderPlugin` and the `CommandDescriptor` catalog still use
  inventory and are unaffected — every catalog entry references its crate by
  path, so the crate stays linked.

* **Which identity an MCP server acts under was a side effect of its auth mode,
  not a stated property.** `api_key` happened to be org-wide, `oauth` happened to
  be per-user, and `agent_identity_connections` silently shadowed
  `user_connections` whenever a session carried an identity - so the account a
  write landed under could change with session wiring, with no config change and
  no event saying so. There was also no way to express "the agent itself", which
  is what scheduled work and the Linear case need. `agent-mcp-attachments.md`
  proposes `actsAs` (`none`/`service`/`user`) as an explicit, fail-closed
  property of each attachment, demotes org MCP servers to presets that supply
  transport and OAuth client registration only, and restricts unattended runs to
  `service` so they stop borrowing whichever human the session resolved to.

* **The Framework had no harness, so every embedding application rebuilt one by
  hand and no two agreed.** `everruns` collapses harness, agent, and session into
  one builder and synthesizes an anonymous harness per session, so a built-in
  world such as `generic` exists once for the platform and again as an ad-hoc
  builder chain in each application, and an agent carries its own shell and
  filesystem capabilities, which means moving it to a container edits the agent.
  A proposal promotes the harness to an application-facing value holding a
  required environment, a capability set, and model defaults, with no base prompt
  and no starter files: a written world-description drifts from the world, and the
  preamble should derive from `ComputeCapabilities` instead. Requirements are
  checked against the environment at session creation rather than discovered by
  the model at the first tool call. The name was re-examined against the prior art
  and kept; no other product has this middle layer, because no other product
  treats capabilities as composable modules.
* **Nothing made an agent stop before a consequential action, and the one gate
  that could was opt-in and unusable by default.** `tool_approval` blocks per
  call and needs a host that can service an interactive prompt, so a hosted
  session had no confirmation layer at all and Platform Chat's confirmation
  rules were prose with no way to actually pause. The new `soft_approval`
  capability is the other shape of the problem: prompt guidance that asks the
  model to batch safe work and stop only at destructive or outward-facing
  actions, with the pause expressed as a `request_approval` tool call so it is
  renderable and auditable rather than a turn that appears to have died. It is
  on by default at `normal` for the `generic` and `platform-chat` harnesses,
  shares the `ApprovalMode` vocabulary with the hard gate, and takes its level
  from a host-supplied store when the host owns one, which is the seam a
  terminal host needs to adopt it without losing its own setting. See
  [Soft Approval](execution/soft-approval.md).

* **A granted approval said what was approved but never who approved it.** The
  grant was a tool call in the session event log, which is the right record of
  the conversation and the wrong thing to answer to: it is scoped to one
  session, and nothing in it names a person. Having the model write an
  `approved_by` would have been worse, an identity claim by the thing being
  governed. The tools now stamp only the turn and input message the consent was
  spoken in, and the server resolves the approver from the authenticated
  initiator it already writes onto that message, emitting
  `agent.approval.requested` / `agent.approval.granted` to the org audit log. A
  turn with no human initiator is recorded as unattributed rather than dropped,
  because an approval nobody granted is the finding. See
  [Soft Approval](execution/soft-approval.md).

* **Two audit variants no consumer could observe were classified as a breaking
  release.** `AgentAction` is public in the published `everruns-platform`, and at
  `0.x` the minor is the breaking slot, so adding `ApprovalRequested` /
  `ApprovalGranted` demanded `0.24.1 -> 0.25.0` plus patch re-pins for thirteen
  published dependants that changed nothing. Single-versioning has since removed
  that bookkeeping, so the cascade is no longer the reason to care. The enum is
  `#[non_exhaustive]` for the reason that outlived it: an external `match` on it
  cannot be broken by a future audit action, which is the break `LlmErrorKind`
  inflicted twice. No `_` arm was needed, since every match on `AgentAction`
  lives in the defining crate. See
  [Release Process](project/release-process.md).

## 2026-09-16

* **Demo screenshots are maintained product assets, not disposable PR evidence.** The canonical set
  now covers Platform Chat, Sessions, Agents, Harnesses, and Durable Execution in matching light and
  dark frames. Captures preserve a 1440-by-900 CSS viewport while rendering 2880-by-1800 HiDPI
  assets. Their scene data, framing constraints, refresh policy, and reproducible capture entry point
  are recorded in [Demo Screenshot Set](ui/demo-screenshots.md); transient UI review evidence remains
  upload-only.

* **Blueprint config schemas were hand-written JSON that nothing validated.**
  Each blueprint carried a `json!` schema duplicating the shape, bounds, and
  defaults its Rust code already knew, and the spawn path only checked whether
  config was present when the schema had required properties - so a declared
  bound was advice to the model, not a constraint on the host. Schemas are now
  derived from a typed config struct through the same derivation that backs
  typed capability tool schemas, and the spawn path validates host config
  against the derived schema before creating a child session. The bounds reuse
  the constants the blueprint's tools already clamp against, so schema and
  runtime cannot drift.

* **Slack approvals, task progress, and the second-token problem are one
  missing capability, not three features.** All three reduce to an agent being
  unable to act on its own Slack channel with that channel's identity. The
  approval half needs no new protocol: `setup_connection` and `url_elicitation`
  already establish pause-and-consent via [Client
  Hints](runtime-resources/client-hints.md), and Slack becomes a third client of
  it — including the degradation path, which is exactly today's behaviour when a
  surface cannot draw the card. Task state is already on `ToolContext`; the gap
  is rendering, and it belongs in the delivery adapter rather than in model
  narration. Recorded as [Slack Agent
  Actions](integrations/slack-agent-actions.md), which also settles that an
  approval click binds to both the pending tool call and an identified Slack
  user, defaulting to the requester.

## 2026-09-15

* **Platform Chat v2 is behind an org-opt-in feature flag.** It is a different way
  to do what the current surface already does, not a deployment capability an
  operator runs, so `platform_chat_v2` is `experimental` and not
  `platform_managed`: an org admin turns it on, and `for_org` keeps it off until
  they do even where the deployment allows it. The gate reads on list and on
  session creation rather than at provisioning, because built-ins are seeded by
  name when an org is created and gating there would leave an org that enables
  the flag later without the harness until something re-provisioned. Hiding is
  not a control on its own — a harness id is stable and guessable — so selecting
  a gated harness is rejected as well. See
  [Platform Chat v2](harnesses/platform-chat.md).

* **Per-crate versioning was bumping more crates per release, not fewer, and the
  cause was additive change being classified as breaking.** At `0.x` the minor is
  the breaking slot, so adding an enum variant or a struct field to a base crate
  forces a minor bump, which no dependant's caret admits, which republishes the
  whole publish cone - 41 of 41 published crates in `0.27.0`, only 8 of them
  carrying a real contract change. The churn-prone public types now carry
  `#[non_exhaustive]`, which makes those additions patch-sized and, separately,
  stops them hard-breaking external consumers' `match`es. The measured limit is
  recorded too: replayed over `0.19.0`-`0.27.0` this avoids one cascade outright
  and shrinks one, because the rest came from genuine API removals. See
  [Release Process](project/release-process.md).

* **Slack had no manual test cases, and it is the reference messaging
  integration.** Four cases covered what the integration suite could not reach:
  Slack manifest acceptance, progressive pane replies, safe status lines,
  Markdown rendering, and terminal notices. The cases depended on the retired
  App setup UI and were removed with that surface. See
  [Messaging Integrations](integrations/messaging-integrations.md).
* **The command line is one contract, shared by the CLI and the agent-facing
  tree.** Deriving the agent-facing parser from each command's JSON Schema
  produced a parallel contract, not the same one: `--system_prompt` where the
  CLI ships `--system-prompt`, no short options where it ships `-f -H -t -a -s
  -o`, a different positional shape. `everruns-cli-contract` now holds the
  grammar as data and owns the one `clap::Command` builder both surfaces call; a
  command declares the part a schema cannot know (short options, bare words,
  worked examples) beside itself, and the rest comes from the schema. The
  shipped CLI's spellings are pinned by a golden snapshot, so where the two
  disagree the surface with users does not move. Turning on the guard that
  parses every documented example found that most of them did not run. The CLI
  mounts the contract commands it does not hand-write, dispatching them through
  the method and path each already declares, so it grew from 44 to 82 commands
  without a hand-written implementation for any of them and without a single
  shipped spelling changing. See [Command tree](execution/command-tree.md).

* **A leaf's flags are parsed by clap, compiled from the schema the command
  already publishes.** The tree hand-parsed `--flag value` pairs, which kept an
  unknown flag as a string property and passed it on, so `--limti 10` became a
  silently dropped argument; required fields surfaced as deserialization errors
  from the far side of a dispatch; and a positional had to be faked by rewriting
  the command string before the interpreter saw it. `CliCommandSpec` now carries
  the command's JSON Schema, a leaf compiles into a `clap::Command`, and the
  parse and the `--help` are generated from that one declaration. `crates/cli`
  cannot lend its definition: it is clap derive over the SDK with client-side
  work of its own. What is shared is the parser and its conventions. Note that
  `clap::Command` is a runtime builder over owned strings, which is why a tree
  assembled from specs fetched at runtime is possible where `CliRoute`, being
  `&'static`, is not. See [Command tree](execution/command-tree.md).

* **`everruns` is a builtin of the agent's own shell, by forwarding rather than a
  local tree.** The tree in `integrations/bashkit/src/cli.rs` resolves in-process,
  and a hosted worker cannot do that: `CliRoute` is `&'static`, so a tree cannot
  be rebuilt from specs fetched at runtime, and the commands live behind the
  control plane. A tool that already accepts a script now declares a
  `CliSpelling` and the shell installs a builtin that hands it the rendered
  command line, keeping grammar, help, authorization, and error shaping where
  they already are. The builtin is installed from the session's tool registry, so
  it re-spells a surface the session already has rather than granting one: a
  harness that withholds the capability withholds the command, with no capability
  list to keep in sync. See [Platform Chat v2](harnesses/platform-chat.md).

* **Memory mounts are live, not snapshots.** `memory.md` always specified
  write-through, but the implementation copied a Memory's files into
  `session_files` at session creation, so every session was a private fork: a
  note written in one was invisible to the next and died with the session. The
  server-managed mounts (`/memory/agent`, `/memory/user`, and the new
  `/memory/shared`) now resolve per access against `memory_files`. Resolution is
  derived from the session row, so it survives a restart with no mount table and
  a workspace without a session row of its own resolves to no mounts, which is
  the privacy boundary for `/memory/user`. Capability-configured `mounts[]` still
  snapshot. See [Memory](runtime-resources/memory.md).

* **Proposed Platform Chat v2: one Bashkit shell instead of three bespoke tools.**
  v1 runs bash without a filesystem, so its `discover`/`query`/`execute` split is a
  toolset boundary rather than a permission one, and its rules live in ~4 KB of prompt
  prose. v2 composes what already exists — `bashkit_shell` over the session filesystem,
  the `everruns` command tree, the virtual `/docs` mount, and a Memory mounted
  read-write at `/memory`. Two real gaps block it: no host inserts the
  `CliCommandSourceHandle` that installs the CLI builtin, and Memory mounts are
  snapshots copied into `session_files` at session creation rather than the
  write-through the spec promises, so concurrent chat threads cannot share memory at
  all. Proposed, not implemented. See [Platform Chat v2](harnesses/platform-chat.md).

* **Platform Chat memory is both shared and private, as sibling mounts.**
  The two are not alternatives: the runtime already mounts `/memory/agent` beside
  `/memory/user`, and the private path's boundary is already enforced end to end.
  v2 reuses `/memory/user` verbatim and adds `/memory/shared`, one memory per
  (org, surface), resolved by reserved name so no new scope or migration is
  needed. They are siblings rather than overlays because the spec rejects
  overlapping mounts; precedence is resolved in the disclosed index instead.
  Writes default to private, and promotion to shared is an explicit user act,
  because a shared note has been read by other people's threads and cannot be
  taken back. See [Platform Chat v2](harnesses/platform-chat.md).

## 2026-09-14

* **Decided to retire the App abstraction in favor of agent-owned exposure.**
  App's required `harness_id` duplicates the agent's, its `agent_id` is nullable only to
  grandfather pre-agent rows, and its publish switch is wider than any single exposure —
  while everything App genuinely contributes is contributed by its channel rows. The
  proposal re-homes channels as agent-owned Endpoints, folds webhook invocation into the
  existing Agent Triggers, unifies the two session-routing enums, and moves ingress URLs
  onto the endpoint id so nothing installed breaks. App is hidden from the product
  surface first and its table deleted last. Accepted, not implemented; the phases are
  tracked as separate OSS issues. See [Agent Exposure](integrations/agent-exposure.md).

* **The Slack channel is now a Slack agent app, not a classic Events API bot.**
  Slack had shipped a dedicated agent surface — split-view container, app threads,
  native streaming, session status, suggested prompts — plus manifest fields that
  carry the event subscription URL, and we used none of it. Eleven changes closed
  most of that gap. The load-bearing decision: the surface is chosen per inbound
  event, not by configuration, so one app serves both the assistant pane and channel
  threads, `report_progress_only` is scoped to channels rather than retired, and the
  pane forces per-thread routing while channels keep their configured strategy.
  Streaming made the delivery dispatcher stateful and clocked, which is why correct
  terminal-state handling had to land first: an unstopped stream is worse than the
  silence it replaces. Reasoning in
  [Slack Integration Modernization](integrations/slack-modernization.md).

* **An issue that links to unmerged context has no context.** Thirteen issues for the
  Slack work were filed pointing at a knowledge concept that had not been merged, so
  every `Context:` link 404d for the whole time the work was being picked up. The
  concept lands first, or in the same change, or the reasoning goes in the issue body.
  Recorded as a filing rule in [Issue Tracking](project/issue-tracking.md).

## 2026-09-11

* **Session schedules are now safe to poll from more than one server
  instance.** `claim_due_session_schedules` selected due rows with
  `FOR UPDATE SKIP LOCKED` but ran the statement on the pool, so the implicit
  transaction committed and released the row locks before the caller advanced
  `next_trigger_at`, and nothing recorded that a row had been picked up. Two
  instances both saw the same schedules as due and both fired them: duplicate
  agent turns, duplicate monitor probes, duplicate model spend. `mark_triggered`
  was no backstop either, being an unguarded read-then-write with no
  compare-and-swap. The claim is now a single atomic statement that stamps
  `claimed_by`/`claimed_at`, the shape `durable_schedules` already used, and it
  is a lease so an instance that dies mid-fire does not strand its schedules.
  Both storage backends implement it. Found while validating a downstream
  zero-downtime rollout that wanted two server replicas.

* **Test fixtures live with the crate that owns them, and never ship.** The
  root `testdata/` and `tests/` trees each held a single fixture set. Plugin
  marketplace fixtures moved to `crates/core/testdata/plugins/` (core owns the
  plugin file set and compiler; host and server tests reach it through
  `../core/testdata/plugins`), and the downstream-consumer workspace moved to
  `crates/everruns/tests/fixtures/external-consumer/`, still its own cargo
  workspace outside the repository workspace. Both crates now declare `exclude`
  so neither fixture tree inflates a published package. The vestigial
  `proposals/` relocation stub was removed; migration
  `087_eval_external_runs.sql` is immutable, so its comment link to the old
  path is left dangling on purpose, and the canonical design is
  [External results publishing](evaluation/external-results-publishing.md).

* **Manual test cases became knowledge.** `test_cases/` moved to
  `knowledge/test-cases/`, so cases are OKF concepts reached by the same
  progressive disclosure as every other concept: domain index, target index,
  feature index, case. Each case carries `type: Test Case` frontmatter with its
  title and a one-sentence description, and every folder has an index. The
  format specification moved from `evaluation/test-cases.md` to
  [Test Cases Specification](test-cases/format.md). Manual run results now land
  outside the bundle, in `.local/test-results/`.

## 2026-09-09

* **Where commands run, and what they may touch.** Everruns' sandbox providers
  answered only the first question and Yolop's only the second, so neither could
  express "run on this machine but deny the network" or "no sandbox at all". The
  proposed environment model makes target and containment orthogonal fields of
  one profile, keeps a single model-facing toolset across Bashkit, Daytona,
  Docker, a registered machine, and the host, and lets an agent pick among
  preapproved environments rather than authoring one. Named Environment on both
  surfaces, with a domain model separating the pinned profile from the durable
  Environment row and its disposable instances. The Framework's existing
  `Environment` seam is the anchor: it already carries a workspace head and an
  extension point documented for compute. Includes proposed Framework and HTTP
  API shapes. See
  [Sandbox Templates](harnesses/sandbox-templates.md).

* **One command grammar, several hosts.** Operations reached through the
  scripted MCP surface, a session's shell, and (eventually) the external CLI had
  drifted into two spellings. The `everruns <noun> <verb>` tree is declared once
  per command and rendered by every host, with the flat wire name kept as the
  identity so dispatch, schema coercion, policy, and error handling are
  unchanged. A tree also makes `--help` affordable where a flat namespace of
  hundreds of commands had to forbid it. See
  [Command Tree](execution/command-tree.md).

## 2026-09-08

* The Framework live-session example now offers offline and live OpenAI modes,
  configurable corrections, and transcript inspection. Its iteration-boundary
  `Session::send` behavior remains distinct from the isolated WebSocket steering
 experiment. See [the steering boundary](execution/openai-steering-prototype.md).

## 2026-09-05

* Astra reasoning changes preserve the initial request effort, persist effective
  effort through stream recovery and checkpoints, and use explicit Responses
  compaction so configuration updates do not break long conversations. See
  [Compaction](runtime-resources/compaction.md).

* Integration capabilities can share their protocol and vendor operation across
  Framework and hosted execution while binding credentials in each context.
  Brave Search establishes the application adapter pattern and separates hosted
  connector registration from the Framework dependency graph. See
  [Framework application boundaries](framework/application-api.md).
* Connected opt-in native async tools to normal Reason/Act execution with shared
  encrypted journals, worker ownership fencing, transcript retention, cancellation,
  and original-call continuation gating. Live Astra function/custom acceptance
  passed on 2026-09-06; ambiguous receipt recovery remains fail-closed. See
  [Native asynchronous tool calls](execution/native-async-tools.md).

* **Mid-turn correction justifies revisiting socket ownership.** The isolated
  OpenAI steering prototype uses a durable inbox and explicit single-host owner.
  Acceptance remains queued until a successor is created; ambiguous disconnects
  block replay until history proves commitment. Normal workers continue over
  HTTP. See `knowledge/execution/openai-steering-prototype.md`.

## 2026-09-01

* **A model switch is a conversation event, not a setting.** Changing the model
  mid-session silently changes every answer after it, but the only record was
  `llm.generation`, which is diagnostic and off the reading path.
  `session.model.changed` now marks the switch in the transcript, carrying names
  captured at emission time so an old session stays readable after a rename.
  Only explicit override → explicit override transitions are reported: an
  inherited default cannot be named against capability-filtered history, and
  guessing it would produce phantom switches. See
  `knowledge/execution/events.md`.

* **A conversation event belongs where the runtime decides, not where one API
  writes.** The first cut emitted the model switch from the server's
  message-create path, which left the in-process framework runtime — a peer host
  that writes its own `input.message` — silent. Detection moved to the host's
  reason path, the single place both hosts resolve a turn's model. The cost is
  ordering: the marker now follows the input message instead of preceding it.
  See `knowledge/execution/events.md`.

## 2026-08-25

* **A projection applied at one read boundary is not applied at the others.**
  Reasoning replay state (`signature`, `encrypted`) was stripped from
  `GET .../messages` but not from `GET .../events`, which serves the same
  message inside `output.message.completed` — so one endpoint withheld what the
  other published. The projection now lives on `EventData` and is applied on both
  the list and SSE paths. Storage keeps the state: replay rebuilds the message
  from the event log, so the projection belongs at the API edge, not at write
  time. See `knowledge/execution/events.md`.

* **A singleton session is a surface, not a capability.** `POST /v1/sessions/chat`
  and `POST /v1/sessions/chat/voice` existed to resolve one Platform Chat session
  per user by tag. Chats binds each thread to an agent through the ordinary
  session routes, which left both endpoints without a caller — and a per-user
  singleton is a shape the rest of the API does not have. Retired with the
  command, service method and harness-name plumbing they were the only readers
  of. The Platform Chat harness and the `global-chat` tag on existing sessions
  are unchanged. See `knowledge/execution/apis.md`.

* **Reasoning is a list of provider artifacts, not text on the message.** The
  flat `thinking` / `thinking_signature` pair could not express what providers
  actually emit: Anthropic signs each thinking block separately and interleaved
  thinking produces several per response, OpenAI keys reasoning items by an id it
  issues, and Gemini binds a thought signature to one function call. All three
  requirements are about *position and identity*, which a single per-message
  field erases. Reasoning is now `ContentPart::Reasoning`, ordered in
  `Message.content`. See `knowledge/foundations/llm-drivers.md`.

* **A reasoning summary is reasoning, not commentary.** OpenAI's summary stream
  was mapped to assistant text, which persisted it as the model's answer and
  replayed it as the model's own prior output. Channel assignment is not
  cosmetic: it decides what gets stored and what the model is told it said. See
  `knowledge/execution/events.md`.

* **Two provider opt-ins are silent when missing.** OpenAI returns
  `encrypted_content` only when the request asks for it via `include`, and Gemini
  returns thought parts only when `thinkingConfig` sets `includeThoughts`.
  Without them the model still reasons and nothing errors, but nothing is
  replayable and nothing reaches the reasoning channel. Advertising reasoning
  support in a model profile is a claim about the driver, not the model.

* **`phase` needs a source.** For every provider without native phase support,
  "commentary" is computed from tool-call presence and carries no independent
  meaning, so a text-only preamble is classified as a final answer. Consumers
  cannot see that from the value alone, so the completed message now publishes
  `phase_source` (`provider` or `derived`) beside it.

## 2026-08-24

* **Output retention is not a recovery affordance.** Persistence-enabled tools
  retain non-empty output in the session filesystem, but expose model-facing
  recovery paths only for content absent from the inline result. Complete output
  must not induce a redundant file-tool round.

* **Filesystem discovery has one batch owner.** The filesystem capability can
  read a bounded ordered set of independent known paths in one structured call,
  while dependent paths remain sequential. Every item still crosses the same
  host filesystem boundary, preserving mount routing, containment, per-file
  truncation, and one aggregate output ceiling.

## 2026-08-22

* **A scheduled session's GitHub identity is not negotiable, so "try another
  token" is never the fix.** The agent egress proxy rewrites `Authorization`
  for `api.github.com`: an invalid token and no token both authenticate as the
  session's own GitHub App installation. Measured 2026-08-22. That closes
  EVE-926's first exit criterion — granting the Doppler PAT Dependabot-alert
  access cannot work, because the PAT is discarded in transit — and leaves
  widening the App installation as the only path. See
  `knowledge/security/security-testing.md`.

* **Session tab badges are counters, not counts.** The session detail tab bar
  renders on every page load, so Work, Events and Workspace are badged from
  denormalized columns maintained by statement-level triggers
  (`sessions.event_count`, `sessions.task_count`, `workspaces.file_count`),
  never from an aggregate over `events`. The file counter lives on `workspaces`
  because files were rekeyed to the workspace in migration 056 and a workspace
  can back more than one session. Zero is reported as absent so an empty tab
  renders unannotated. See `knowledge/operations/session-counts.md`.

## 2026-08-21

* **Dependency removal is measured, not estimated, and two traps produce wrong
  answers.** Sibling dependencies mask each other's cost — `ethers-signers`
  alone measured 11 crates, the ethers stack together measured 64 — and
  dev-dependency edges are not shipped, which is why OpenSSL appeared to be a
  runtime dependency when it only entered through a bench. See
  `knowledge/project/dependency-surface.md`.

* **Owning a small, fully specified contract can beat both libraries.** x402
  EIP-712 signing moved in-tree on `k256` after `alloy-signer-local` measured
  worse than the retired `ethers-rs` it would replace (103 exclusive crates
  against 64). The override of the "do not own crypto" default is licensed by a
  known-answer vector proving byte-equivalence, not by preference.

* **Prometheus durations are histograms, not summaries.** The in-tree recorder
  renders `_bucket`/`_sum`/`_count`. Summary quantiles cannot be aggregated
  across replicas, which contradicted the documented horizontal-scaling model.

* **Web fetch no longer renders JavaScript in-process.** `render=rakers` is
  gone; browserless and deno serve rendered pages. TM-TOOL-024 is mitigated by
  removal.

## 2026-08-17

* **Crate boundaries follow ownership rather than package count.** Public,
  kernel, deployment, and selectable integration boundaries remain focused
  crates; forwarding-only leaves should fold into their owner. Official model
  drivers are grouped physically under `crates/drivers/` while retaining
  independent package identities and versions over `everruns-provider`.

* **Session services are host-owned.** `SessionMutator` and the portable
  session/session-storage capabilities are collocated in `everruns-host`;
  platform re-exports the same types. The former leaf package added release
  overhead without an independent runtime boundary.

## 2026-08-15

* **Production simulation has a focused owner.** The deterministic LLM driver,
  scripted-turn configuration, registry helpers, and optional host-builder
  extension live in the publishable `everruns-llmsim` crate. Framework and
  worker production graphs depend on it directly; `everruns-test-support`
  retains testing/demo helpers and a documented 0.18 compatibility re-export
  for its 0.17 simulator paths.

* **Framework architecture is public and unambiguous.** The Framework guide
  now presents concrete `everruns::Engine` as the canonical application API,
  separates it from the low-level `everruns-engine::Execution` host contract,
  and maps immediate and durable execution onto their shared turn kernel. The
  persistence guide distinguishes volatile, local crash-durable, and
  distributed Platform recovery boundaries. The diagram contract now lives in
  the documentation knowledge domain and permits at most two restrained,
  labeled semantic accents when color clarifies an important boundary.

* Moved the remaining active design work out of the temporary `proposals/`
  area and into its owning knowledge domains: secret-leak guardrails under
  security, external evaluation publishing under evaluation, and the portable
  sandbox abstraction under harnesses. Implemented Platform proposals were
  removed rather than preserved as historical design documents.

* **Framework execution is concrete and backend-owning.** `everruns::Engine`
  owns Agent snapshots, its session catalog, and the backend bundle used by
  every bound Session. `InMemoryEngine` is only a compatibility alias and the
  private Session execution binding is not an application SPI. Live Engines
  configured for the same local profile share one backend cell, preventing
  independent JSONL indexes or SQLite handles from diverging inside a process.

* **The host no longer implies the control plane.** At this stage, the neutral
  session-services package owned `SessionMutator` plus the portable session
  and session-storage capabilities. Platform re-exported that boundary for
  product consumers, while host and the default Framework graph used it
  directly. Platform composition was an opt-in host feature, and the Resend
  client is an opt-in platform feature; dependency guards reject platform,
  Reqwest, Rustls, or Hyper in minimal host/Framework graphs.

* **Execution persistence has one state-overlay authority.** Both immediate
  and durable hosts apply the effects returned by `everruns-engine`; failures
  to record required lifecycle status/events now fail the transition. The
  worker persists `DurableExecution::checkpoint()` at reason, act, wait, and
  completion boundaries instead of reconstructing resume fields on its wire
  path. Local JSONL recovery is bounded before indexing (128 MiB and 1,000,000
  events by default).

* **Remaining control-plane records and policy left the kernel where no turn
  consumes them.** Stored `Budget` and `LedgerEntry` records live in platform;
  core retains only budget execution vocabulary. Schedule limit values remain
  neutral defaults in core, while environment-variable policy is resolved by
  the local and server adapters.

## 2026-08-14

* **The 0.18 kernel/API freeze established a boundary, not a size target.**
  EVE-906 removed compatibility ownership and credential-bearing resolved
  values, pinned the reviewed public surface, and kept neutral contracts that
  portable turn execution still consumes. The freeze guard must be updated
  deliberately when a missed product-only record moves to its real owner; it
  is not a reason to preserve an acknowledged layering mistake.

* **Engine-owned sessions, multi-head workspaces, and shared execution landed
  as one model.** Framework Engines own session identity and Environment head
  binding. `everruns-engine` owns the shared Input/Reason/Act algorithms and
  pure turn planner. Immediate and durable hosts select persistence and
  scheduling, but do not carry independent copies of turn semantics.

* **The portable distributed-engine experiment was added and reverted.** The
  short-lived `everruns-scale` crate combined a public registered-agent API
  with another execution composition. It was reverted because Everruns needs
  one shared abstract execution/turn kernel in `everruns-engine`, with
  in-process and durable hosts adapting that kernel, not a third Scale product
  layer or a second embedded Engine path. The subsequent unification work is
  the retained implementation of that intent.

## 2026-08-13

* **EVE-897 closed at two families: the `ToolContext` service bag mostly cannot
  be dismantled.** `session_sqldb` and `session_mutator` now resolve as typed
  extensions; the other 17 families stay. The reason is structural rather than
  effortful, and is the durable finding here.

  A capability reads `context.storage_store` today, a field on core's
  `ToolContext`. Moving `SessionStorageStore` to platform turns that into
  `context.extensions.get::<SessionStorageStoreExt>()`, and naming that wrapper
  requires depending on `everruns-platform`. But platform already depends on
  those crates: `environment-capabilities` pulls in filesystem, bashkit, lua,
  web-fetch and openrouter-workspace, and `portable-builtins` pulls in
  `everruns-builtins`. So `consumer -> platform -> consumer` is a dependency
  cycle, which Cargo rejects outright.

  Of the 17 remaining families, 16 have a consumer below platform. Four are
  reached from `everruns-builtins`, four from core itself (where the direction
  is the epic's foundation), and the rest through integration crates, five of
  which platform depends on, making those cycles too. Only the nine
  non-cycle integrations could take a new edge, and that means every
  integration crate pulling all of platform to name a wrapper type.

  The issue assumed these optional fields were hosted services leaking into the
  kernel. Most are neutral contracts that portable code legitimately needs
  during a turn, independently the same conclusion EVE-880 reached about
  `session_schedule`. `ToolContext`'s width is a symptom of many hosted
  services existing, not of them living in the wrong crate.

  The structural fix, not taken: a neutral contracts crate below builtins,
  integrations and platform. Worth revisiting only if the bag becomes a
  concrete maintenance problem rather than an aesthetic one.

  Practical test for the next person asking "should this leave core?", check
  the crates *below* platform, not just core's own consumers. Core-side
  cleanliness is not evidence a move is possible.

  `session_task_registry` is the one clean family left unmoved.

* **Kernel dependency hygiene (EVE-888).** Removed two vestigial features from
  `everruns-core`: `sqlx` (zero usage in the crate; it only forwarded to
  `everruns-provider/sqlx`, where the typed-ID Postgres impls live, the server
  now depends on provider directly) and `embedded-platform-docs` (gated nothing;
  `include_dir` was unused and the real embedding moved to platform with
  EVE-839). Core is down to 15 direct dependencies. A manifest-wide sweep
  matching each declared dependency against source identifiers found no others,
  so this vein is exhausted.

  `utoipa` stays, and needs no work: it is already `optional = true` behind an
  `openapi` feature absent from core's defaults. Core's default build resolves
  zero utoipa crates; the 183 `ToSchema` derives are all
  `#[cfg_attr(feature = "openapi", ...)]` and compile away unless a consumer
  opts in. EVE-888's "remove OpenAPI derives" bullet is satisfied as written.

## 2026-08-12

* **EVE-880 closed: three session families stay in the kernel by design.**
  `Workspace`, the managed sandbox and `session_sqldb` moved to
  `everruns-platform`. `session_task`, `session_schedule` and
  `session_resource` stay in `everruns-core`, and the reason is the same in
  each case: a portable, kernel-resident consumer needs the contract during a
  turn.

  - `session_task`, `wake_queue` decides mid-turn wakes from the task's wake
    policy, `task_observer` is the lifecycle SPI, and the record is serialized
    whole into the canonical `task.created` / `task.updated` /
    `task.message.*` payloads.
  - `session_schedule`, `crates/builtins/src/usage_limit_auto_continue.rs`
    reads `ctx.services.schedule_store` to schedule an auto-resume after a
    provider usage limit. `everruns-builtins` depends only on
    `everruns-capability` and `everruns-core`; platform depends on *it*, so
    moving the contract to platform would put it out of a portable built-in's
    reach. A typed extension does not help, the wrapper would live in
    platform, equally invisible.
  - `session_resource`, `resource_ownership.rs` and the skills capabilities
    in `crates/core/src/capabilities/`, which are portable and stay.

  Core already owns neutral store contracts of this kind (`SessionFileSystem`,
  `SessionStorageStore`); these belong with them. The generalisable rule, worth
  carrying into EVE-888: whether something is a platform record is answered by
  *who consumes it during a turn*, not by whether it is persisted. All three
  families that stay are persisted, and all three are essential for
  portable execution.

* **Background tool runs leave the kernel**: Moved `spawn_background`, the
  tool, its session-task mirroring, the scheduled-monitor path, the background
  event sink, admission-control permits and the reattach entry point, out of
  `everruns-core` into `everruns-platform` as `background_run` (EVE-888,
  ~1800 lines). Creating session tasks and schedules is hosted behaviour; the
  kernel keeps the neutral `BackgroundExecutableTool`/`BackgroundEventSink`
  contracts in `core::background` and runs whatever a host supplies. The
  `background_execution` capability, which already lived in platform, now owns
  the tool it advertises, and `subagents` shares the same admission permits so
  every detached path goes through one gate.

  This did **not** free the `session_task` record for EVE-880, contrary to the
  expectation recorded against EVE-897. Three consumers remain in core, and one
  is essential: `SessionTask` and `TaskMessage` are embedded in the
  canonical `task.created` / `task.updated` / `task.message.*` event payloads
  (`events.rs`), which EVE-888 explicitly retains as kernel surface while
  putting changes to canonical event semantics out of scope. `wake_queue.rs`
  and `task_observer.rs` also consume the record. Moving the family therefore
  needs a neutral task projection for events, or an accepted event-payload
  change, a decision, not a mechanical move. `session_schedule` is separately
  pinned by `SessionScheduleStore` in `core::traits`, which is the EVE-897
  pattern.

* **Session SQL store becomes a typed context extension**: Removed
  `ToolContext::sqldb_store` and moved the whole `session_sqldb` family,
  store trait, value types and error, from `everruns-core` to
  `everruns-platform` (EVE-897, first family). The field was the only thing
  pinning the family to the kernel: core named the trait, the trait's
  signatures named the value types, and no core execution path touched either.
  The capability now resolves `SessionSqlDbStoreExt` from the type-keyed
  extension bag core already carried, and the host installs it beside the
  other typed services. `SessionSqlDbStoreRef` and the
  `ToolContextService::SessionSqlDbStore` variant are gone. The capability
  never declared this service in `required_context_services`, so a missing
  store still surfaces as the same structured tool error.

  The remaining ~18 optional service fields (~1250 call sites) follow one
  family per change. This boundary frees `session_sqldb` only. (An earlier version
  of this entry expected `session_task`, `session_schedule` and
  `session_resource` to follow under EVE-888; they do not, see the EVE-880
  closeout below.)

* **Composition root extraction**: Moved `PlatformDefinition` out of
  `everruns-core` into `everruns-host` as `HostComposition` (EVE-887).
  Selecting which capabilities, drivers and host services a deployment runs
  with is composition, not kernel execution configuration, so the bundle now
  belongs to the layer that executes a turn; core keeps the registries and
  service contracts it carries. The fields stay owned by their layers
  (driver registry from `everruns-provider`, capability registry from the
  neutral capability contract, egress and utility LLM from their own
  contracts), no central enum and no vendor branching. Product presets keep
  inventory discovery confined: the server's OSS preset is
  `oss_host_composition`, the worker's is `default_host_composition`, and the
  Framework facade builds its private in-process host from the same focused
  type without importing either preset. A new core guard fails the build if a
  composition root reappears in the kernel under any name.

* **Session sandbox record extraction**: Moved the managed per-session sandbox
, config, persisted state, provider instance and exec/file payloads, the
  `SessionSandboxProvider` SPI with its inventory plugin, and the
  create/resume/pause/delete/init/checkpoint lifecycle helpers, out of
  `everruns-core` into `everruns-platform` (EVE-880), where the sandbox
  capability already lives after EVE-886. One provider-backed sandbox per
  session is control-plane state: a turn reaches it through the capability,
  never through the kernel. Integration providers (Daytona) register against
  platform. The agent-record isolation guard now covers the sandbox record
  types and the provider SPI, and the `Workspace` row moved earlier in the
  same issue.

  `session_sqldb` stayed in core in this change. Its value types are the
  signature vocabulary of `SessionSqlDbStore`, and that trait was pinned to
  core by `ToolContext::sqldb_store`; splitting records from trait would have
  made core name platform types. (That family moved under EVE-897, not
  EVE-887 as this entry first said, see the EVE-897 entry above.) The same
  reasoning held for the session task, schedule and resource records, which
  still have execution-time consumers inside core.

## 2026-08-11

* **Hosted capability extraction**: Moved hosted knowledge, Memory,
  delegation, background/scheduled task, user-hook, citation, model-scout,
  OpenRouter-workspace, and platform-management capability implementations out
  of `everruns-core` into `everruns-platform` (EVE-885). Core keeps neutral
  capability/tool/task/event/delegation contracts, generic collection hooks,
  and type-keyed service extensions. Product presets explicitly compose the
  hosted registry; the Framework preset no longer advertises capabilities whose
  persistence, tenancy, worker, or authorization services are absent.

* **Connection/auth/email infrastructure extraction**: Moved the hosted
  connector catalog (`Connector` trait, `ConnectorRegistry`,
  `ConnectorPlugin` inventory registration) and the system email contract
  with its concrete senders (`EmailSender`, templates, `SystemEmailConfig`,
  `ResendEmailSender`) out of `everruns-core` into `everruns-platform`, and
  the OAuth 2.1 protocol client (`OAuthClient`, `TokenSet`, PKCE, discovery,
  form-encoded token exchange) into `everruns-mcp`, its only consumer,
  as `everruns_mcp::oauth::protocol` (EVE-879, breaking for direct core
  consumers in 0.18). `PlatformDefinition` no longer carries a connector
  registry or email sender; server composition owns both
  (`ServerAppBuilder::connector_registry` / `::email_sender`, OSS presets in
  `crates/server/src/platform.rs`). The `CredentialProvider` boundary moved to
  `everruns-provider` (re-exported by core unchanged). Secret-bearing types
  (`TokenSet`, `PkcePair`, `ProviderCredentials`, `ResendEmailConfig`) now
  redact credentials in `Debug`, with tests. Core dropped its
  `serde_urlencoded` and `eventsource-stream` dependencies; REST/gRPC shapes
  unchanged (OpenAPI byte-identical); the agent-record isolation guard now
  also covers connector/OAuth/email types.

* **Eval/observer/feature-management record extraction**: Moved the persisted
  eval aggregates (definitions, runs, results, dataset exports, targets,
  scorers), the observer records (match rules, judge configuration,
  trace-score lifecycle), and the org/product feature-flag records with their
  management catalog out of `everruns-core` into `everruns-platform`
  (EVE-878, breaking for direct core consumers in 0.18). These are product
  management/reporting aggregates that never participate in a turn. Core
  keeps only `execution_features`, `InternalFeatureFlags` and the resolved
  `ExecutionFeatureDecisions` snapshot consulted at capability-registration
  time, while per-org effective feature decisions are resolved server-side
  and applied by filtering the capability list handed to the worker.
  REST/OpenAPI shapes and stored schema are unchanged; the agent-record
  isolation guard now also covers eval/observer/feature-management records.
  Updated evals, online-evals, citations, and feature-flags references.

* **Session aggregate extraction**: Moved the persisted `Session`
  database/API aggregate, product status/source/activity facets,
  participants, ownership references, previews, timestamps, catalog
  relationships, out of `everruns-core` into `everruns-platform` (EVE-882,
  breaking for direct core consumers in 0.18). Core keeps only the portable
  `ExecutionSession` (correlation values plus the session configuration
  overlay a turn consumes) and the neutral `SessionExecutionState`; the
  stored `SessionStatus` maps to/from it at the adapter boundary, and host
  status mutation acknowledges without returning a record. Server
  repositories and worker adapters project the stored record via
  `Session::execution_session()` at the loading boundary; embedded/local hosts
  lift the execution view back into a minimal record with
  `Session::from_execution_session()` only where they implement platform
  boundaries. REST/gRPC and stored schema are unchanged (OpenAPI byte-identical);
  the agent-record isolation guard now also covers Session records.

* **Harness record extraction**: Moved the stored `Harness` persistence
  record, lifecycle status, hierarchy identifiers, built-in flags, display
  metadata, timestamps, chain-merge helpers, and the built-in provisioning
  templates (`BuiltInHarnessDefinition`, roles) out of `everruns-core` into
  `everruns-platform` (EVE-881, breaking for direct core consumers in 0.18).
  Core keeps only the portable `HarnessDefinition` (effective environment
  configuration); the `HarnessStore` loading boundary resolves parent-chain
  inheritance and enforces archived/deleted validation before host execution,
  so hosts never request or receive a stored Harness. Built-in harness
  composition moved off `PlatformDefinition` onto server composition
  (`ServerAppBuilder::built_in_harnesses`). REST/gRPC and stored schema are
  unchanged (OpenAPI byte-identical); the agent-record isolation guard now
  also covers Harness records.

* **Provider SPI separation completed**: Official wire-protocol provider
  crates no longer depend on `everruns-core` on any edge kind, the last
  dev-dependencies were removed and their tests now build fixtures in-crate
  (EVE-874). A new architecture guard
  (`scripts/lib/check-provider-isolation.sh`, pre-push + CI) forbids direct
  core/host/platform/server dependencies from provider crates and keeps heavy
  core feature subtrees out of provider-only builds; a downstream provider
  fixture proves custom drivers compile against `everruns-provider` alone.
  Updated code-organization.

* **Agent record extraction**: Moved the stored `Agent` and `AgentVersion`
  persistence records, lifecycle status, versioning and publication metadata,
  fork lineage, public-name validation, and persistence helpers, out of
  `everruns-core` into `everruns-platform` (EVE-877, breaking for direct core
  consumers in 0.18). Core keeps only the portable `AgentDefinition` (authored
  execution configuration); the `AgentStore` loading boundary projects stored
  records into it and enforces archived/deleted validation before host
  execution, which consumes the resolved execution snapshot only. REST/gRPC
  and stored schema are unchanged, and a new architecture guard keeps kernel
  crates (core, engine, provider, capability) free of platform record imports.

## 2026-08-10

* **Observability extraction**: Moved telemetry initialization (OTLP exporter
  wiring, tracing-subscriber layers, `TelemetryConfig`/`TelemetryGuard`) and
  the `CompositeEventListener` fan-out out of `everruns-core` into
  `everruns-observability` (EVE-876). Core keeps only the neutral observability
  contracts, the `EventListener` trait, event types, and gen-AI span
  conventions, and carries no OpenTelemetry/exporter dependencies; a new
  architecture guard enforces the isolation and keeps Framework/provider
  builds free of the exporter subtree.

* **Test-support extraction**: Moved deterministic simulation and demo-only
  behavior out of `everruns-core` into the new `everruns-test-support` crate
  (EVE-875): the `llmsim` driver, the in-memory agentic loop, mock test
  doubles, and the fake/demo fixture capabilities. Core has no llmsim
  dependency or feature, product registries no longer register demo
  capabilities, and a new architecture guard enforces the isolation. Updated
  code-organization, capabilities, LLM-driver, sans-IO, and agent-handoff
  concepts accordingly.

* **Single low-level host boundary**: Deleted the `everruns-runtime`
  compatibility crate for 0.18 and retired the runtime compatibility and
  deprecation specification. `everruns-host` is now the only low-level host
  boundary: ordinary applications use `everruns`, advanced hosts use `everruns`
  plus `everruns-host` and focused siblings, and Framework, embedding,
  provider, capability, Lua, and code-organization knowledge name it directly.

## 2026-08-09

* **Live Framework sessions**: Made asynchronous message acceptance the primary
  session contract, with atomic active-turn steering, authoritative routing
  receipts, optional waiting, and request/response as convenience.

* **Framework knowledge ownership**: Defined the application-facing purpose,
  canonical Framework/Runtime/SDKs/Platform terminology, open provider
  boundary, library-experience success bars, and documentation/example contract;
  reframed the foundations runtime specification as low-level 0.17.x host
  compatibility.

* **Framework application boundary**: Established the Framework knowledge
  collection and classified workspace, MCP, plugins, context inspection,
  event-derived history/resume, and schedules as application concerns while
  retaining writable message stores, backend topology, mount primitives, and
  host orchestration as low-level `0.17.x` compatibility surfaces.

## 2026-08-08

* **Navigation information architecture**: Recorded the placement rule that groups the
  shell by what you do with a thing (Chats, Operational, Building, Registries, Quality),
  the worked hard cases, the surface contracts, and the three dismissed options.

* **Agent MCP credentials**: Added durable write-only Agent bindings for MCP
  tool-parameter credentials, model-schema removal, runtime-only injection,
  secure setup affordances, and tenant/non-disclosure threat controls.

* **Platform resource grounding**: Distinguished operation discovery from
  authoritative entity reads, added user-scoped connection preflight, and
  required Platform Chat to report installed, available, attached, and connected
  integration state independently before reusable-resource confirmation.

## 2026-08-07

* **Slate fidelity**: Retired the handwritten experimental page stamp; experimental
  navigation now uses the single Lucide flask marker defined by the design system.

* **Platform behavioral eval**: Replaced the legacy tool-name study with a
  live-server Mira eval for `platform` command sequencing, safety, loop budgets,
  and persisted hourly Agent/MCP/model/trigger state.

## 2026-08-06

* **Platform command surface**: Added the high-risk built-in `platform`
  capability with MCP-parity `discover`, read-only `query`, and mutating
  `execute` tools. Platform Chat now uses the shared command inventory, and the
  worker transport re-establishes the session owner's authorization server-side.

* **Main synchronization**: Migrated the newly landed Sans-IO turn-state and WebMCP
  specifications into the OKF bundle and incorporated their feature-flag and threat-model updates.
* **Migration**: Moved the canonical `specs/` corpus into an OKF v0.2 bundle under
  `knowledge/`, preserving the specifications' semantics while adding concept metadata
  and domain indexes.
* **Enforcement**: Added a local conformance and link checker plus the upstream
  `okf-lint` CI gate. Maintenance rules are recorded in the
  [Knowledge Maintenance Contract](knowledge-contract.md).
