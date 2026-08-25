# Everruns Knowledge Update Log

## 2026-08-25

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
