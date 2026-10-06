---
title: Capability integrations
description: Select filesystem, shell, web, Lua, and MCP implementation boundaries without pulling them into the Everruns kernel.
---

The Framework separates capability contracts from environment-backed
implementations. `everruns-core` defines capability, tool, filesystem, egress,
and MCP invocation contracts; focused crates own code that touches an
interpreter, network transport, local process, or session filesystem.

This keeps a custom host's dependency and trust boundaries visible in
`Cargo.toml`. It also prevents a core registry from silently granting an
execution or network surface.

## Framework features

| `everruns` feature | Default | Implementation | Effect boundary |
|---|---:|---|---|
| `filesystem` | Yes | `everruns-integrations` (`filesystem` feature) | Host-provided, session-scoped filesystem only |
| `bashkit` | No | `everruns-integrations` (`bashkit` feature) | Sandboxed shell; HTTP remains capability-config and egress-policy gated |
| `web-fetch` | No | `everruns-integrations` (`web-fetch` feature) | FetchKit requests through the host egress contract |
| `lua` | No | `everruns-integrations` (`lua` feature) | Vendored Lua 5.4 sandbox; also requires `FEATURE_LUA=prod` at runtime |
| `mcp` | No | `everruns-core` (`mcp` feature) | Remote HTTP MCP through the host egress contract |
| `mcp-stdio` | No | `everruns-core` (`mcp` feature) | Adds local-process MCP servers and implies `mcp` |
| `host-shell` | No | `everruns-core` (`host` feature) | `bash` tool over real host processes, bounded by a kernel policy; implies `host-compute` |

| `duckduckgo` | No | `everruns-integrations` (`duckduckgo` feature) | DuckDuckGo web search through the host egress contract |
| `a2a` | No | `everruns-capabilities` | Outbound A2A delegation to remote agents; implies `local` |
| `otel` | No | `everruns-core` (`host` feature) | OpenTelemetry exporter; see [Observability](/observability/) |
| `braintrust` | No | `everruns-core` (`host` feature) | Braintrust exporter; see [Observability](/observability/) |

The default is offline: the filesystem capability can only use the
session-filesystem implementation supplied by the host. Shell, web, Lua, MCP,
and local-process transports require explicit features.

```toml
[dependencies]
everruns = { version = "0.34", features = ["bashkit", "web-fetch"] }
```

Enabling an implementation does not activate it on every agent. Add the
matching capability reference to the agent, and retain the documented role,
network-access, and runtime feature gates. In particular, Bashkit, web fetch,
and Lua remain high-risk capabilities in the hosted product.

## Provider integrations

Integration packages can expose typed values through `IntoCapability`. Brave
Search supports the ordinary Framework builder:

```rust
use everruns::{Agent, OpenAI};
use everruns_integrations::brave_search::BraveSearch;

let agent = Agent::builder()
    .instructions("Search and cite primary sources.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .capability(BraveSearch::from_env()?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Depend on `everruns-integrations` with `default-features = false` and the
`brave-search` feature. Add `brave-search-integration` to opt into Platform
connector registration; leave it off for a Framework-only dependency. The
Framework adapter reads
`BRAVE_SEARCH_API_KEY` at construction; `BraveSearch::new` accepts an explicit
application-owned key. Keys are retained privately by the client, never placed
in capability JSON. Hosted execution continues to resolve connections and
session secrets at tool execution time. Both paths use the same search schema
and operation. Framework calls use the application's direct HTTP client.

## Advanced host composition

Advanced embedders select integrations on `everruns` and build the
runtime registry through `everruns::batteries::runtime_capability_registry()`:

```toml
[dependencies]
everruns-core = "0.34"
everruns = { version = "0.34", default-features = false, features = ["filesystem", "web-fetch"] }
```

```rust
let registry = everruns::batteries::runtime_capability_registry();
let egress = everruns::batteries::runtime_egress_service();
assert!(registry.has("session_file_system"));
assert!(registry.has("web_fetch"));
assert!(!registry.has("bashkit_shell"));
# let _ = egress;
```

If the host starts from a caller-owned registry, preserve it and apply the same
feature-selected integrations with
`everruns::batteries::compose_runtime_capability_registry(registry)`.

Hosted server and worker composition uses
`everruns_capabilities::capabilities::hosted_capability_registry_for_grade` with
the platform's `environment-capabilities` feature. That preset preserves the
hosted catalog while keeping the implementations outside core.

Depend directly on a focused crate when you need its public implementation
types. The former core paths move as follows:

| Former public path | New public path |
|---|---|
| `everruns_core::FileSystemCapability` and filesystem tools | `everruns_integrations_filesystem::*` |
| `everruns_core::BashkitShellCapability`, `BashTool`, and adapter | `everruns_integrations_bashkit::*` |
| `everruns_core::WebFetchCapability`, `WebFetchTool`, and bot-auth helpers | `everruns_integrations_web_fetch::*` |
| `everruns_core::LuaCapability` and `LuaCodeModeCapability` | `everruns_integrations_lua::*` |
| `everruns_core::McpCapability` and MCP capability-ID helpers | `everruns_core::mcp::*` |
| `everruns_core::DirectEgressService` | `everruns_core::host::DirectEgressService` with `direct-egress` |
| `everruns_core::SystemEmailConfig` and Resend types | `everruns_capabilities::*` |
| `everruns_core::ModelScoutCapability` and `OpenRouterWorkspaceCapability` | `everruns_integrations_openrouter::*` |
| `everruns_core::OpenRouterServerToolsCapability` | `everruns_integrations_openrouter::OpenRouterServerToolsCapability` |
| `everruns_core::{HumanIntentCapability, InfinityContextCapability, SkillsCapability, AttachSkillCapability, ToolApprovalCapability}` | `everruns_core::builtins::*` |
| `everruns_core::{OpenUiCapability, A2UiCapability}` | `everruns_core::builtins::*` with `ui-capabilities` |
| `everruns_core::skill::ProcessCommandExecutor` | `everruns_core::host::ProcessCommandExecutor` with the host `process` feature |

Continue with [Configure and author capabilities](/framework/advanced-capabilities/)
for agent-level activation or [Custom backends](/framework/custom-backends/)
for host-level storage and orchestration.
