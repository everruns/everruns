# Everruns Knowledge Update Log

## 2026-08-12

* **EVE-880 closed: three session families stay in the kernel by design.**
  `Workspace`, the managed sandbox and `session_sqldb` moved to
  `everruns-platform`. `session_task`, `session_schedule` and
  `session_resource` stay in `everruns-core`, and the reason is the same in
  each case: a portable, kernel-resident consumer needs the contract during a
  turn.

  - `session_task` — `wake_queue` decides mid-turn wakes from the task's wake
    policy, `task_observer` is the lifecycle SPI, and the record is serialized
    whole into the canonical `task.created` / `task.updated` /
    `task.message.*` payloads.
  - `session_schedule` — `crates/builtins/src/usage_limit_auto_continue.rs`
    reads `ctx.services.schedule_store` to schedule an auto-resume after a
    provider usage limit. `everruns-builtins` depends only on
    `everruns-capability` and `everruns-core`; platform depends on *it*, so
    moving the contract to platform would put it out of a portable built-in's
    reach. A typed extension does not help — the wrapper would live in
    platform, equally invisible.
  - `session_resource` — `resource_ownership.rs` and the skills capabilities
    in `crates/core/src/capabilities/`, which are portable and stay.

  Core already owns neutral store contracts of this kind (`SessionFileSystem`,
  `SessionStorageStore`); these belong with them. The generalisable rule, worth
  carrying into EVE-888: whether something is a platform record is answered by
  *who consumes it during a turn*, not by whether it is persisted. All three
  families that stay are persisted, and all three are load-bearing for
  portable execution.

* **Background tool runs leave the kernel**: Moved `spawn_background` — the
  tool, its session-task mirroring, the scheduled-monitor path, the background
  event sink, admission-control permits and the reattach entry point — out of
  `everruns-core` into `everruns-platform` as `background_run` (EVE-888,
  ~1800 lines). Creating session tasks and schedules is hosted behaviour; the
  kernel keeps the neutral `BackgroundExecutableTool`/`BackgroundEventSink`
  contracts in `core::background` and runs whatever a host supplies. The
  `background_execution` capability, which already lived in platform, now owns
  the tool it advertises, and `subagents` shares the same admission permits so
  every detached path goes through one gate.

  This did **not** free the `session_task` record for EVE-880, contrary to the
  expectation recorded against EVE-897. Three consumers remain in core, and one
  is load-bearing: `SessionTask` and `TaskMessage` are embedded in the
  canonical `task.created` / `task.updated` / `task.message.*` event payloads
  (`events.rs`), which EVE-888 explicitly retains as kernel surface while
  putting changes to canonical event semantics out of scope. `wake_queue.rs`
  and `task_observer.rs` also consume the record. Moving the family therefore
  needs a neutral task projection for events, or an accepted event-payload
  change — a decision, not a mechanical move. `session_schedule` is separately
  pinned by `SessionScheduleStore` in `core::traits`, which is the EVE-897
  pattern.

* **Session SQL store becomes a typed context extension**: Removed
  `ToolContext::sqldb_store` and moved the whole `session_sqldb` family —
  store trait, value types and error — from `everruns-core` to
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
  family per change. This seam frees `session_sqldb` only. (An earlier version
  of this entry expected `session_task`, `session_schedule` and
  `session_resource` to follow under EVE-888; they do not — see the EVE-880
  closeout below.)

* **Composition root extraction**: Moved `PlatformDefinition` out of
  `everruns-core` into `everruns-host` as `HostComposition` (EVE-887).
  Selecting which capabilities, drivers and host services a deployment runs
  with is composition, not kernel execution configuration, so the bundle now
  belongs to the layer that executes a turn; core keeps the registries and
  service contracts it carries. The fields stay owned by their layers
  (driver registry from `everruns-provider`, capability registry from the
  neutral capability contract, egress and utility LLM from their own
  contracts) — no central enum and no vendor branching. Product presets keep
  inventory discovery confined: the server's OSS preset is
  `oss_host_composition`, the worker's is `default_host_composition`, and the
  Framework facade builds its private in-process host from the same focused
  type without importing either preset. A new core guard fails the build if a
  composition root reappears in the kernel under any name.

* **Session sandbox record extraction**: Moved the managed per-session sandbox
  — config, persisted state, provider instance and exec/file payloads, the
  `SessionSandboxProvider` SPI with its inventory plugin, and the
  create/resume/pause/delete/init/checkpoint lifecycle helpers — out of
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
  EVE-887 as this entry first said — see the EVE-897 entry above.) The same
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
  form-encoded token exchange) into `everruns-mcp` — its only consumer —
  as `everruns_mcp::oauth::protocol` (EVE-879, breaking for direct core
  consumers in 0.18). `PlatformDefinition` no longer carries a connector
  registry or email sender; server composition owns both
  (`ServerAppBuilder::connector_registry` / `::email_sender`, OSS presets in
  `crates/server/src/platform.rs`). The `CredentialProvider` seam moved to
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
  keeps only `execution_features` — `InternalFeatureFlags` and the resolved
  `ExecutionFeatureDecisions` snapshot consulted at capability-registration
  time — while per-org effective feature decisions are resolved server-side
  and applied by filtering the capability list handed to the worker.
  REST/OpenAPI shapes and stored schema are unchanged; the agent-record
  isolation guard now also covers eval/observer/feature-management records.
  Updated evals, online-evals, citations, and feature-flags references.

* **Session aggregate extraction**: Moved the persisted `Session`
  database/API aggregate — product status/source/activity facets,
  participants, ownership references, previews, timestamps, catalog
  relationships — out of `everruns-core` into `everruns-platform` (EVE-882,
  breaking for direct core consumers in 0.18). Core keeps only the portable
  `ExecutionSession` (correlation values plus the session configuration
  overlay a turn consumes) and the neutral `SessionExecutionState`; the
  stored `SessionStatus` maps to/from it at the adapter boundary, and host
  status mutation acknowledges without returning a record. Server
  repositories and worker adapters project the stored record via
  `Session::execution_session()` at the loading seam; embedded/local hosts
  lift the execution view back into a minimal record with
  `Session::from_execution_session()` only where they implement platform
  seams. REST/gRPC and stored schema are unchanged (OpenAPI byte-identical);
  the agent-record isolation guard now also covers Session records.

* **Harness record extraction**: Moved the stored `Harness` persistence
  record — lifecycle status, hierarchy identifiers, built-in flags, display
  metadata, timestamps, chain-merge helpers — and the built-in provisioning
  templates (`BuiltInHarnessDefinition`, roles) out of `everruns-core` into
  `everruns-platform` (EVE-881, breaking for direct core consumers in 0.18).
  Core keeps only the portable `HarnessDefinition` (effective environment
  configuration); the `HarnessStore` loading seam resolves parent-chain
  inheritance and enforces archived/deleted validation before host execution,
  so hosts never request or receive a stored Harness. Built-in harness
  composition moved off `PlatformDefinition` onto server composition
  (`ServerAppBuilder::built_in_harnesses`). REST/gRPC and stored schema are
  unchanged (OpenAPI byte-identical); the agent-record isolation guard now
  also covers Harness records.

* **Provider SPI separation completed**: Official wire-protocol provider
  crates no longer depend on `everruns-core` on any edge kind — the last
  dev-dependencies were removed and their tests now build fixtures in-crate
  (EVE-874). A new architecture guard
  (`scripts/lib/check-provider-isolation.sh`, pre-push + CI) forbids direct
  core/host/platform/server dependencies from provider crates and keeps heavy
  core feature subtrees out of provider-only builds; a downstream provider
  fixture proves custom drivers compile against `everruns-provider` alone.
  Updated code-organization.

* **Agent record extraction**: Moved the stored `Agent` and `AgentVersion`
  persistence records — lifecycle status, versioning and publication metadata,
  fork lineage, public-name validation, and persistence helpers — out of
  `everruns-core` into `everruns-platform` (EVE-877, breaking for direct core
  consumers in 0.18). Core keeps only the portable `AgentDefinition` (authored
  execution configuration); the `AgentStore` loading seam projects stored
  records into it and enforces archived/deleted validation before host
  execution, which consumes the resolved execution snapshot only. REST/gRPC
  and stored schema are unchanged, and a new architecture guard keeps kernel
  crates (core, engine, provider, capability) free of platform record imports.

## 2026-08-10

* **Observability extraction**: Moved telemetry initialization (OTLP exporter
  wiring, tracing-subscriber layers, `TelemetryConfig`/`TelemetryGuard`) and
  the `CompositeEventListener` fan-out out of `everruns-core` into
  `everruns-observability` (EVE-876). Core keeps only the neutral observability
  contracts — the `EventListener` trait, event types, and gen-AI span
  conventions — and carries no OpenTelemetry/exporter dependencies; a new
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
