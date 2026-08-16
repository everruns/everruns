---
title: Migrate to 0.18
description: Move Rust code off `everruns-core` paths that changed in 0.18, with a symbol-by-symbol table of where each type now lives.
---

0.18 narrows `everruns-core` to the neutral execution kernel. Types that were persisted control-plane records, hosted service contracts, product composition or concrete integrations moved to the crate that owns them. The behaviour, the wire formats and the stored schema are unchanged, only the import paths.

## Retain an Engine for sessions

The Framework exposes a concrete application execution owner. The 0.18 API
removes `agent.session()` and `agent.resume(id)`; applications retain the
engine that owns session identity and resume authority:

```diff
- let session = agent.session();
+ use everruns::Engine;
+ let engine = Engine::new();
+ let session = engine.create(agent);
  let id = session.session_id();
- let resumed = agent.resume(id).await?;
+ let resumed = engine.resume(id).await?;
```

`Engine` volatile resume is deliberately process-local and engine-scoped. The
old `InMemoryEngine` name remains a type alias. Engine retains the immutable
Agent snapshot and exact Environment/WorkspaceHead; it
does not serialize a Scale-compatible Agent definition.

For a locally persisted session after process restart, rebuild the Agent from
trusted application configuration, call `engine.attach(id, agent).await?`, and
then `engine.resume(id).await?`. Attachment verifies the persisted session
catalog before accepting the behavior snapshot; credentials and closures are
never serialized.

This affects you if your Rust code imports from `everruns_core` directly. If you use the `everruns` facade, most of this is invisible: the facade re-exports what applications need, and where a moved type is part of that surface it is re-exported from its new home under the same name.

## The quickest path

Most migrations are a find-and-replace of a crate prefix. Compile, read the unresolved-import errors, and look each symbol up in the tables below.

```bash
cargo build 2>&1 | grep -E "unresolved import|no .* in"
```

Add whichever crates the table points you at:

```toml
everruns-platform = "0.18"   # persisted records, hosted service contracts
everruns-host     = "0.18"   # execution composition and host wiring
everruns-provider = "0.18"   # provider SPI, typed IDs, sqlx impls
everruns-capability = "0.18" # capability identity/configuration contract
everruns-session-services = "0.18" # neutral session host contracts
everruns-mcp      = "0.18"   # MCP adapter and the OAuth protocol client
everruns-llmsim   = "0.18"   # deterministic production-safe simulator
```

## Composition

The single biggest change for embedders. `PlatformDefinition` no longer exists.

| 0.17 | 0.18 |
|---|---|
| `everruns_core::PlatformDefinition` | `everruns_host::HostComposition` |
| `everruns_core::PlatformDefinitionBuilder` | `everruns_host::HostCompositionBuilder` |
| `everruns_server::oss_platform_definition()` | `everruns_server::oss_host_composition()` |
| `everruns_server::oss_platform_definition_for_grade()` | `everruns_server::oss_host_composition_for_grade()` |
| `everruns_worker::default_platform_definition()` | `everruns_worker::default_host_composition()` |
| `ServerAppBuilder::platform_definition(..)` | `ServerAppBuilder::host_composition(..)` |
| `WorkerAppBuilder::platform_definition(..)` | `WorkerAppBuilder::host_composition(..)` |

```diff
- use everruns_core::PlatformDefinition;
+ use everruns_host::HostComposition;

- let platform = PlatformDefinition::builder()
+ let composition = HostComposition::builder()
      .capability_registry(capabilities)
      .driver_registry(drivers)
      .build();

- ServerAppBuilder::new().platform_definition(platform)
+ ServerAppBuilder::new().host_composition(composition)
```

The type is otherwise identical, same fields, same builder methods. It moved to the layer that executes a turn, because selecting a deployment's capabilities and drivers is composition rather than kernel configuration.

### Input/Reason/Act execution kernel

Concrete phase execution moved out of `everruns-core`. Import phase algorithms
and their I/O values from `everruns-engine`; keep neutral effect contracts in
core.

| 0.17 | 0.18 |
|---|---|
| `everruns_core::atoms::{InputAtom, InputAtomInput, InputAtomResult}` | `everruns_engine::{InputAtom, InputAtomInput, InputAtomResult}` |
| `everruns_core::atoms::{ReasonAtom, ReasonInput, ReasonResult}` | `everruns_engine::{ReasonAtom, ReasonInput, ReasonResult}` |
| `everruns_core::atoms::{ActAtom, ActInput, ActResult, ToolCallResult}` | `everruns_engine::{ActAtom, ActInput, ActResult, ToolCallResult}` |
| `everruns_core::atoms::AtomContext` | `everruns_core::ExecutionContext` (also re-exported by `everruns-engine`) |
| `everruns_core::atoms::{PreToolUseHook, PostToolExecHook, PreToolUseDecision}` | `everruns_core::tool_hooks::*` |

The generic `Atom` trait was removed; it had no production dynamic-dispatch
use. Call the concrete executor's inherent async `execute` method. There is no
core compatibility module in 0.18. Serialized phase payloads retain the same
fields, so durable records remain readable even though the Rust ownership path
changed.

### Turn context and command completion

Store-backed turn preparation now belongs to `everruns-host`. Core keeps the
secret-free execution snapshot, pure context transformations, and narrow
effects used by custom hosts.

| 0.17 | 0.18 |
|---|---|
| `everruns_core::assemble_turn_context` | `everruns_host::assemble_turn_context` |
| `everruns_core::inspect_turn_context` | `everruns_host::inspect_turn_context` |
| `everruns_core::load_execution_snapshot` | `everruns_host::load_execution_snapshot` |
| `everruns_core::load_execution_snapshot_for_session` | `everruns_host::load_execution_snapshot_for_session` |
| `everruns_core::StoreCommandHost` | `everruns_host::StoreCommandHost` |

`everruns_engine::ReasonAtom::new` no longer accepts harness, agent, session, and provider stores
or a driver registry. Construct an `everruns_host::StoreTurnContextResolver`
from those host services, then pass that resolver plus the narrow message,
capability, and event effects to the atom. Hosts that already loaded a
`ResolvedExecutionSnapshot` should call
`everruns_host::assemble_turn_context_from_snapshot` and execute the atom with
the resulting `AssembledTurnContext`; this avoids a second store load.

For a fully custom host, implement the neutral
`everruns_core::TurnContextResolver`, or provide already-resolved
`ResolvedTurnContextInput` to `everruns_core::assemble_resolved_turn_context`.
That input contains a secret-free model/provider identity and an opaque ready
driver; provider keys and endpoints are never serializable kernel values.

`CommandTurnContext` now exposes `session_id` directly instead of an
`ExecutionSession`. Commands retain the same filtered messages, effective
prompt, locale, model, streaming, and error-classification behavior without
receiving a session record.

## Persisted records

These are database and API records. Execution consumes a portable projection of each; the stored row is control-plane state.

| 0.17 (`everruns_core::`) | 0.18 |
|---|---|
| `Agent`, `AgentVersion`, `AgentStatus`, `AgentVersionChangeKind` | `everruns_platform::` |
| `Harness`, `HarnessStatus`, `BuiltInHarnessDefinition`, `BuiltInHarnessRole` | `everruns_platform::` |
| `Session`, `SessionStatus`, `SessionSource`, `SessionActivity`, `SessionParticipant` | `everruns_platform::` |
| `Workspace`, `WorkspaceStatus` | `everruns_platform::workspace::` |
| `Eval`, `EvalCase`, `EvalRun`, `EvalCaseResult`, `EvalRunDataset`, `EvalTarget`, `Scorer` | `everruns_platform::` |
| `Observer`, `ObserverMatch`, `LlmJudgeConfig`, `TraceScore` | `everruns_platform::` |
| `FeatureFlags`, `FeatureFlagMap`, `FeatureFlagDefinition` | `everruns_platform::` |
| `Budget`, `LedgerEntry` | `everruns_platform::` |

If you were reading a stored record to run a turn, you probably want the portable projection instead, `AgentDefinition`, `HarnessDefinition` and `ExecutionSession` all stay in `everruns_core`, produced at the platform loading boundary by `Agent::execution_definition`, `Harness::execution_definition` and `Session::execution_session`.

## Hosted service contracts

| 0.17 (`everruns_core::`) | 0.18 |
|---|---|
| `session_sqldb::*`, `SessionSqlDbStore`, `DatabaseInfo`, `SqlQueryResult`, `SqlExecuteResult`, `TableSchema`, `ColumnSchema`, `SessionSqlDbError` | `everruns_platform::session_sqldb::` |
| `traits::SessionMutator` | `everruns_session_services::SessionMutator` (also re-exported by platform) |
| `session_sandbox::*`, config, state, instance, exec/file payloads, `SessionSandboxProvider`, `SessionSandboxProviderPlugin` | `everruns_platform::session_sandbox::` |
| `Connector`, `ConnectorRegistry`, `ConnectorPlugin` | `everruns_platform::connector::` |
| `EmailSender`, `EmailMessage`, `SystemEmailConfig`, `ResendEmailSender` | `everruns_platform::email::` |
| `OAuthClient`, `TokenSet`, `PkcePair` | `everruns_mcp::oauth::protocol::` |

Neutral per-turn contracts remain in `everruns-core`, but the catch-all
`everruns_core::traits` module is gone. Import from the owning concern instead,
for example `everruns_core::tool_context::ToolContext`,
`everruns_core::session_files::SessionFileSystem`, or
`everruns_core::provider_resolution::ProviderStore`. The deployment-owned
`SessionFileSystemFactory` and its context now come from `everruns-host`.

Two of these also changed how a capability *reaches* the service. `sqldb_store` and `session_mutator` are no longer fields on `ToolContext`; they resolve from the type-keyed extension bag:

```diff
- let Some(store) = &context.sqldb_store else { ... };
+ let Some(store) = context.extensions.get::<SessionSqlDbStoreExt>() else { ... };
+ let store = &store.0;
```

If you implement a custom host, install them the way `everruns-host` does:

```rust
extensions.insert(Arc::new(SessionSqlDbStoreExt(store)));
extensions.insert(Arc::new(SessionMutatorExt(mutator)));
```

## Capabilities and implementations

| what | 0.18 home |
|---|---|
| Knowledge Bases and Indexes, Memories, delegation, subagents, background and scheduled work, user hooks, citations, model scouting, platform management | `everruns_platform::capabilities::` |
| Session info and session storage | `everruns_session_services::capabilities::` (also re-exported by platform) |
| Session SQL database and session sandbox | `everruns_platform::capabilities::` |
| `spawn_background` and its runtime, event sink, admission permits, reattach | `everruns_platform::background_run::` |
| Portable built-ins, human intent, infinity context, skills, UI prompts, compaction, tool search | `everruns_builtins::` |
| OpenRouter workspace, model scout, and provider-executed server tools | `everruns_integrations_openrouter_workspace::` |
| Filesystem, shell, web fetch, Lua | `everruns_integrations_*` |
| MCP adapter | `everruns_mcp::` |
| HTTP transports | `everruns_http::` |
| Telemetry init, exporter event listeners, `CompositeEventListener` | `everruns_observability::` |
| `llmsim` driver, configs, scripted turns, registry helpers, host-builder extension | `everruns_llmsim::` |
| in-memory agentic loop, writable test doubles, fixture capabilities | `everruns_test_support::` |
| `everruns_core::in_memory::{InMemoryAgentStore, InMemoryHarnessStore, InMemorySessionStore, InMemoryProviderStore}` | `everruns_host::{InMemoryAgentStore, InMemoryHarnessStore, InMemorySessionStore, InMemoryProviderStore}` |
| `everruns_core::in_memory::{InMemoryMessageRetriever, InMemoryEventEmitter}` | `everruns_test_support::{InMemoryMessageRetriever, InMemoryEventEmitter}` for isolated deterministic tests |

Product presets compose these explicitly. Core registries are now empty by
default: use `everruns_host::runtime_capability_registry()` for the Framework
preset or `everruns_platform::capabilities::hosted_capability_registry()` for
the hosted product catalog.

Hosted conversation history has no writable message-store replacement. Append
canonical events through `everruns_host::EventLog` / `HostEventEmitter` and
read messages through `EventHistory`. This avoids message/event dual writes and
keeps resume and replay behavior identical across in-memory and durable hosts.

`everruns-test-support` continues to re-export its 0.17 simulator paths during
the 0.18 migration, so existing test suites can upgrade without an immediate
import rewrite. Treat that as a migration bridge: production code, new tests,
and low-level hosts should depend on `everruns-llmsim` directly. The
application-facing `everruns::Model::simulated` and
`Model::simulated_with_config` APIs are unchanged.

## Features

| 0.17 | 0.18 |
|---|---|
| `everruns-core/sqlx` | removed, use `everruns-provider` with `features = ["sqlx"]` |
| `everruns-core/embedded-platform-docs` | removed, it gated nothing; use `everruns-platform/embedded-platform-docs` |
| `everruns-platform/sqlx` | removed, it forwarded to core's and nothing enabled it |
| `everruns-core/llm-tests` | removed, use the `everruns-llm-tests` package for live provider tests |

`everruns-core` now has an empty default feature set. OpenAPI derives remain
available only with `features = ["openapi"]`; structural outlines remain
available only with `features = ["tree-sitter-outlines"]`. Neither subtree is
present in a default core build.

Concrete provider protocol and utility-model implementations also moved to
their effectful owners:

| 0.17 (`everruns_core::`) | 0.18 |
|---|---|
| `OpenAIProtocolChatDriver`, `openai_protocol` | `everruns_provider::` |
| `OpenResponsesProtocolChatDriver`, `openresponses_protocol` | `everruns_provider::` |
| `driver_helpers`, `stream_reconnect` | `everruns_provider::` |
| `OpenAiUtilityLlmService`, `SystemUtilityLlmConfig`, `UTILITY_OPENAI_API_KEY_ENV` | `everruns_host::` with `features = ["utility-openai"]` |

Core no longer initializes Rustls. Provider HTTP clients install their ring
crypto provider when they are first constructed, while server, worker, and CLI
startup owners install it eagerly. Custom binaries that combine TLS stacks can
depend on `everruns-provider` with `features = ["tls-ring"]` and call
`everruns_provider::install_ring_crypto_provider()` once during startup; the
call is idempotent and safe under concurrent initialization.

## Provider and typed-ID imports

Provider-owned modules are no longer compatibility-exported by
`everruns-core`. Low-level consumers must add `everruns-provider` directly.
This keeps credentials and concrete driver assembly out of the neutral kernel
and makes the dependency owner visible in `Cargo.toml`.

There are two common compiler-error shapes:

1. **The module moved to another crate.** Add that crate and change the prefix.
2. **The module stayed public, but its root convenience re-export was
   removed.** Keep the dependency and qualify the symbol through its module.

The second case produces the misleading-looking `no X in the root` error. It
does not necessarily mean the type moved. These replacements are deliberately
literal so they can be applied with ordinary search-and-replace:

| before | after |
|---|---|
| `everruns_core::ProviderStore` | `everruns_core::provider_resolution::ProviderStore` |
| `everruns_core::SessionStore` | `everruns_core::execution_loading::SessionStore` |
| `everruns_core::MessageRetriever` | `everruns_core::message_retriever::MessageRetriever` |
| `everruns_core::SessionFileSystem` | `everruns_core::session_files::SessionFileSystem` |

| 0.17 core path | 0.18 direct path |
|---|---|
| `everruns_core::driver_registry::*` | `everruns_provider::driver_registry::*` |
| `everruns_core::model::*` | `everruns_provider::model::*` |
| `everruns_core::model_profiles::*` | `everruns_provider::model_profiles::*` |
| `everruns_core::model_spec::ModelSpec` | `everruns_provider::model_spec::ModelSpec` |
| `everruns_core::provider::*` | `everruns_provider::provider::*` |
| `everruns_core::runtime_provider::*` | `everruns_provider::runtime_provider::*` |
| `everruns_core::typed_id::*` | `everruns_provider::typed_id::*` |
| `everruns_core::error::*` | `everruns_provider::error::*` |
| `everruns_core::tool_types::*` | `everruns_provider::tool_types::*` |
| `everruns_core::capability_types::{CapabilityId, CapabilityRef, CapabilityError}` | `everruns_capability::{CapabilityId, CapabilityRef, CapabilityError}` |
| `everruns_core::AgentCapabilityConfig` | `everruns_capability::CapabilityRef` |
| core plugin capability ID/validation helpers | the same symbol in `everruns_capability` |
| `everruns_core::ExecutionPhase` or `message::ExecutionPhase` | `everruns_provider::execution_phase::ExecutionPhase` |
| `everruns_core::ToolResultImage` or `tools::ToolResultImage` | `everruns_provider::tool_types::ToolResultImage` |
| other root-level provider symbols | the same root symbol in `everruns_provider` |

The credential-bearing `everruns_core::ResolvedModel` is removed. Store and
transport boundaries now resolve two separate values:

- `ModelSpec`, safe to serialize and pass through the kernel; and
- a host-owned runtime `Provider` (or internal `ProviderConfig`) containing
  endpoint and authentication state.

`ProviderStore::get_model_spec` and `get_default_model_spec` return only the
first value. Hosts obtain provider configuration separately and join it only
while constructing a non-serializable driver/provider execution value.

`ProviderStore::get_provider_config` no longer has a default implementation.
Every custom host must state where credentials live: return its resolved
`ProviderConfig`, or explicitly return `None` when the provider was registered
directly in the host registry or is selected but not configured. A missing
credential no longer prevents turn-context/command assembly; the constructed
driver rejects the first model/list/compact operation locally, before network
I/O. This keeps recovery commands reachable without turning an empty token
into an outbound authorization header.

## Simulator and compaction naming

`LlmSimRuntimeExt::llm_sim` now only registers/replaces the simulator provider.
It never changes the selected model. Existing compact test setups that relied
on implicit selection should use the explicit name:

```diff
- builder.llm_sim(config)
+ builder.llm_sim_as_default(config)
```

When a builder already calls `default_model(...)`, keep `.llm_sim(config)`;
the selected model is preserved regardless of method order.

`everruns_builtins::CompactionConfig` is the sole application-facing policy
builder. The expanded implementation value previously available as
`everruns_builtins::compaction::CompactionConfig` is now
`everruns_builtins::compaction::RuntimeCompactionConfig` (also re-exported at
the crate root). This makes an unqualified `CompactionConfig` unambiguous.

The public core test/backend conveniences are gone as well:

| removed core value | replacement |
|---|---|
| `EchoTool`, `FailingTool` | define the small test `Tool` locally, or use test-support executors |
| `InMemoryCompactionCheckpointStore` | `everruns_host::InMemoryCompactionCheckpointStore` |

## What deliberately did not move

Worth knowing so you do not go looking:

- **`SessionTask`, `TaskMessage`** and the task registry stay in `everruns_core`. They are turn-execution vocabulary, `wake_queue` decides mid-turn wakes from a task's wake policy, and they appear in the canonical `task.created` / `task.updated` / `task.message.*` event payloads.
- **`SessionSchedule`, `SessionScheduleStore`** stay. A portable built-in (`usage_limit_auto_continue`) schedules an auto-resume after a provider usage limit, and it sits below platform in the dependency graph.
- Schedule quota and minimum-interval environment variables are no longer read
  by core. Local/server adapters resolve deployment policy and call the
  parameterized core validation helpers.
- **`SessionResourceRegistry`** stays. `resource_ownership` and the portable skills capabilities consume it.
- **`SessionFileSystem`, `SessionStorageStore`** and the other neutral store contracts stay. Core owns the contract; hosts own the backend.

The rule these follow: whether something belongs in the kernel is decided by whether a portable execution path consumes it during a turn, not by whether it is persisted. All four above are persisted, and all four are essential for execution.

## Getting unstuck

If a symbol is not in these tables, import it from the crate that defines it;
`everruns-core` no longer acts as a compatibility facade for provider-owned
APIs. Framework applications can continue to prefer the higher-level
`everruns` facade. The crate-level docs on `everruns-core` record where each
remaining family lives and why.
