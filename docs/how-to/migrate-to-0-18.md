---
title: Migrate to 0.18
description: Move Rust code off `everruns-core` paths that changed in 0.18, with a symbol-by-symbol table of where each type now lives.
---

0.18 narrows `everruns-core` to the neutral execution kernel. Types that were persisted control-plane records, hosted service contracts, product composition or concrete integrations moved to the crate that owns them. The behaviour, the wire formats and the stored schema are unchanged — only the import paths.

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
everruns-mcp      = "0.18"   # MCP adapter and the OAuth protocol client
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

The type is otherwise identical — same fields, same builder methods. It moved to the layer that executes a turn, because selecting a deployment's capabilities and drivers is composition rather than kernel configuration.

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

If you were reading a stored record to run a turn, you probably want the portable projection instead — `AgentDefinition`, `HarnessDefinition` and `ExecutionSession` all stay in `everruns_core`, produced at the platform loading seam by `Agent::execution_definition`, `Harness::execution_definition` and `Session::execution_session`.

## Hosted service contracts

| 0.17 (`everruns_core::`) | 0.18 |
|---|---|
| `session_sqldb::*` — `SessionSqlDbStore`, `DatabaseInfo`, `SqlQueryResult`, `SqlExecuteResult`, `TableSchema`, `ColumnSchema`, `SessionSqlDbError` | `everruns_platform::session_sqldb::` |
| `traits::SessionMutator` | `everruns_platform::SessionMutator` |
| `session_sandbox::*` — config, state, instance, exec/file payloads, `SessionSandboxProvider`, `SessionSandboxProviderPlugin` | `everruns_platform::session_sandbox::` |
| `Connector`, `ConnectorRegistry`, `ConnectorPlugin` | `everruns_platform::connector::` |
| `EmailSender`, `EmailMessage`, `SystemEmailConfig`, `ResendEmailSender` | `everruns_platform::email::` |
| `OAuthClient`, `TokenSet`, `PkcePair` | `everruns_mcp::oauth::protocol::` |

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
| Session info, session storage, session SQL database, session sandbox | `everruns_platform::capabilities::` |
| `spawn_background` and its runtime — event sink, admission permits, reattach | `everruns_platform::background_run::` |
| Portable built-ins — human intent, infinity context, skills, UI prompts, compaction, tool search | `everruns_builtins::` |
| OpenRouter workspace, model scout, and provider-executed server tools | `everruns_integrations_openrouter_workspace::` |
| Filesystem, shell, web fetch, Lua | `everruns_integrations_*` |
| MCP adapter | `everruns_mcp::` |
| HTTP transports | `everruns_http::` |
| Telemetry init, exporter event listeners, `CompositeEventListener` | `everruns_observability::` |
| `llmsim` driver, in-memory loop, fixture capabilities | `everruns_test_support::` |

Product presets compose these explicitly. Core registries are now empty by
default: use `everruns_host::runtime_capability_registry()` for the Framework
preset or `everruns_platform::capabilities::hosted_capability_registry()` for
the hosted product catalog.

## Features

| 0.17 | 0.18 |
|---|---|
| `everruns-core/sqlx` | removed — use `everruns-provider` with `features = ["sqlx"]` |
| `everruns-core/embedded-platform-docs` | removed — it gated nothing; use `everruns-platform/embedded-platform-docs` |
| `everruns-platform/sqlx` | removed — it forwarded to core's and nothing enabled it |

## What deliberately did not move

Worth knowing so you do not go looking:

- **`SessionTask`, `TaskMessage`** and the task registry stay in `everruns_core`. They are turn-execution vocabulary — `wake_queue` decides mid-turn wakes from a task's wake policy — and they appear in the canonical `task.created` / `task.updated` / `task.message.*` event payloads.
- **`SessionSchedule`, `SessionScheduleStore`** stay. A portable built-in (`usage_limit_auto_continue`) schedules an auto-resume after a provider usage limit, and it sits below platform in the dependency graph.
- **`SessionResourceRegistry`** stays. `resource_ownership` and the portable skills capabilities consume it.
- **`SessionFileSystem`, `SessionStorageStore`** and the other neutral store contracts stay. Core owns the contract; hosts own the backend.

The rule these follow: whether something belongs in the kernel is decided by whether a portable execution path consumes it during a turn, not by whether it is persisted. All four above are persisted, and all four are load-bearing for execution.

## Getting unstuck

If a symbol is not in these tables, `everruns_core` still re-exports a good deal from `everruns-provider` at its original path — `typed_id`, `model`, `error`, `driver_registry`, `tool_types` and others resolve unchanged. For anything else, the crate-level docs on `everruns-core` record where each family went and why.
