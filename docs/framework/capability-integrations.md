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
| `filesystem` | Yes | `everruns-integrations-filesystem` | Host-provided, session-scoped filesystem only |
| `bashkit` | No | `everruns-integrations-bashkit` | Sandboxed shell; HTTP remains capability-config and egress-policy gated |
| `web-fetch` | No | `everruns-integrations-web-fetch` | FetchKit requests through the host egress contract |
| `lua` | No | `everruns-integrations-lua` | Vendored Lua 5.4 sandbox; also requires `FEATURE_LUA=true` at runtime |
| `mcp` | No | `everruns-mcp` | Remote HTTP MCP through the host egress contract |
| `mcp-stdio` | No | `everruns-mcp` | Adds local-process MCP servers and implies `mcp` |

The default is offline: the filesystem capability can only use the
session-filesystem implementation supplied by the host. Shell, web, Lua, MCP,
and local-process transports require explicit features.

```toml
[dependencies]
everruns = { version = "0.17", features = ["bashkit", "web-fetch"] }
```

Enabling an implementation does not activate it on every agent. Add the
matching capability reference to the agent, and retain the documented role,
network-access, and runtime feature gates. In particular, Bashkit, web fetch,
and Lua remain high-risk capabilities in the hosted product.

## Advanced host composition

Advanced embedders select integrations on `everruns-host` and build the
runtime registry through `everruns_host::runtime_capability_registry()`:

```toml
[dependencies]
everruns-core = "0.17"
everruns-host = { version = "0.17", features = ["filesystem", "web-fetch"] }
```

```rust
let registry = everruns_host::runtime_capability_registry();
let egress = everruns_host::runtime_egress_service();
assert!(registry.has("session_file_system"));
assert!(registry.has("web_fetch"));
assert!(!registry.has("bashkit_shell"));
# let _ = egress;
```

If the host starts from a broader core preset, preserve it and apply the same
feature-selected integrations with
`everruns_host::compose_runtime_capability_registry(registry)`.

Hosted server and worker composition uses
`everruns_platform::capabilities::hosted_capability_registry_for_grade` with
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
| `everruns_core::McpCapability` and MCP capability-ID helpers | `everruns_mcp::*` |
| `everruns_core::DirectEgressService` | `everruns_http::DirectEgressService` |
| `everruns_core::SystemEmailConfig` and Resend types | `everruns_platform::*` |
| `everruns_core::ModelScoutCapability` and `OpenRouterWorkspaceCapability` | `everruns_integrations_openrouter_workspace::*` |
| `everruns_core::skill::ProcessCommandExecutor` | `everruns_host::ProcessCommandExecutor` with the host `process` feature |

Continue with [Configure and author capabilities](/framework/advanced-capabilities/)
for agent-level activation or [Custom backends](/framework/custom-backends/)
for host-level storage and orchestration.
