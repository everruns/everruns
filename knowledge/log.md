# Everruns Knowledge Update Log

## 2026-08-11

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
