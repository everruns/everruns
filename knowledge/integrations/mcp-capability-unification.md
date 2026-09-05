---
type: Specification
title: "MCP Capability Unification"
description: "One extensible MCP capability shared by the hosted product and embedding hosts such as yolop: server provider seam, management tools, discovery reuse."
tags:
  - everruns
  - integrations
---
# MCP Capability Unification

> Part of the [MCP spec family](mcp.md). [runtime-mcp.md](runtime-mcp.md) records the
> decision to extract the transport-agnostic client into `everruns-mcp`; that work
> landed. This document records what is *still* divergent between the hosted product
> and downstream embedders (concretely [yolop](https://github.com/everruns/yolop)), and
> the decision to unify it behind one extensible MCP capability.

## Abstract

The MCP **protocol client** is already shared: `everruns-mcp` owns transports,
negotiation, auth resolution, result mapping, and execution, and every host
(server, worker, `everruns-host`, yolop) speaks it. What is **not** shared is
everything wrapped around that client: where the server list comes from, how it is
edited, how it is enabled/disabled and reloaded, and what the agent itself can do
about it. Each host reimplements that layer, and the term "MCP capability" currently
names two unrelated things.

The decision: introduce one **extensible MCP capability** in `everruns-mcp`,
parameterized by a pluggable server-catalog seam, so a host supplies *where servers
are stored* and inherits identity, tools, discovery, enablement, and reload for free.

## Problem state

### The word "capability" names two different things

| Thing | Where | What it is | ID |
|---|---|---|---|
| `everruns_mcp::McpCapability` | `crates/mcp/src/capability.rs` | A **virtual capability per server**: wraps one server's cached `tools/list` result and emits prefixed `ToolDefinition`s with capability attribution. `tools()` returns empty; execution goes through `McpExecutor`. | `mcp:<uuid>` |
| yolop `McpCapability` | yolop `src/capabilities/mcp.rs` | A **management capability**: four agent-facing tools (`list_mcp_servers`, `upsert_mcp_server`, `remove_mcp_server`, `set_mcp_server_enabled`) over a file-backed config store. Contributes no MCP tools. | `mcp` |

They collide on name to the point that yolop's runtime imports its own as
`YolopMcpCapability`. Neither is a superset of the other, and the hosted product has
no management capability at all (its CRUD is REST/gRPC/DB only, in
`crates/server/src/domains/mcp_servers/`), while yolop has no per-server virtual
capability (it never registers one; it only borrows `McpCapability::tool_definitions`
indirectly through the host).

### The catalog layer is reimplemented per host

`ScopedMcpServers` is the shared *value* type, and `merge_scoped_mcp_servers` the
shared merge, but everything that produces and mutates that value is host-private:

| Concern | Hosted product | yolop |
|---|---|---|
| Storage | PostgreSQL rows, org-scoped, archived flag | `settings.toml` `[mcp.servers.*]`, `<config_dir>/yolop/mcp.json`, workspace `.mcp.json`, profile |
| Scope precedence | harness → agent → session (`crates/host/src/mcp.rs::merge_session_scoped_servers`) | global → profile → workspace → ACP `session/new` (`src/config/mcp.rs::load_mcp_servers`) |
| Enable/disable | archived rows, capability attachment | per-entry `enabled: bool`, absent from `ScopedMcpServer` |
| Shape normalization | typed API | `normalize_server_entry_value` (`transport_type` → `type`, `oauth` → `o_auth`, `mcpServers` alias) |
| Secret handling | `secret_bindings` resolved from a secure store | `${VAR}` expansion at load; `secret_bindings` always empty |
| Mutation surface | REST + gRPC + UI | agent tools, `/mcp` command, `yolop mcp` CLI |
| Live reload | per-turn resolution from the DB | `RuntimeHandles::reload_mcp_servers` swapping `session.mcp_servers` |

Two of these are *general* and only accidentally live downstream:
`normalize_server_entry_value` (every `.mcp.json` in the ecosystem uses that shape) and
the per-entry `enabled` flag (the hosted product expresses the same idea with archived
rows).

### Host-private code forces downstream duplication

`crates/host/src/mcp.rs` is `mod mcp;` — not `pub mod`. Everything in it is
`pub(crate)`. Consequently yolop carries byte-level copies:

- `src/runtime/mod.rs::mcp_connection_for` duplicates `host::mcp::endpoint_for` +
  `resolve_servers` (transport match, `McpConnection` construction, empty
  `secret_bindings`).
- `src/runtime/mod.rs::discover_mcp_tool_names` duplicates
  `host::mcp::discover_tool_definitions` minus the cache and concurrency — so `/tools`
  discovery is serial, uncached, and can diverge from what the turn path actually
  offered the model.

Both copies silently drift whenever `ScopedMcpServer` or `McpConnection` gains a field.
`secret_bindings` is the live example: it was added upstream and yolop's copy hardcodes
empty with a comment explaining why.

### Auth is half-shared

`everruns_mcp::oauth::OAuthAuthProvider` is upstream and yolop uses it. But yolop wraps
it in `StoredMcpAuthProvider` = OAuth + a private `EnvMcpAuthProvider` that resolves
`<PROVIDER>_ACCESS_TOKEN` / `_API_KEY` / `_TOKEN` and `MCP_<SERVER>_TOKEN`. That
env-fallback chain is generic headless/CI behavior with nothing yolop-specific in it,
and any other embedder wanting it must rewrite it.

## Decisions

### D1, Rename the per-server virtual capability

`everruns_mcp::McpCapability` becomes `McpServerCapability`, keeping the `mcp:<uuid>`
ID namespace and `McpCapabilityIdExt`. This frees the name `McpCapability` for the
management capability and stops the two concepts reading as one. Internal code needs no
compatibility shim; the rename touches `crates/server/src/services/capability.rs`,
`crates/server/src/domains/mcp_servers/scoped_mcp.rs`, and `crates/host/src/mcp.rs`.

### D2, One `McpCatalog` seam owning storage

Introduce a trait in `everruns-mcp`:

```rust
#[async_trait]
pub trait McpCatalog: Send + Sync {
    /// Every configured server with its scope, enabled state, and origin.
    async fn list(&self) -> Result<Vec<McpCatalogEntry>>;
    /// The effective `ScopedMcpServers` after scope precedence and enablement.
    async fn effective(&self) -> Result<ScopedMcpServers>;
    /// Optional mutation. A read-only catalog returns `Unsupported` and the
    /// management capability omits the write tools.
    async fn upsert(&self, scope: McpCatalogScope, name: &str, server: McpCatalogEntry) -> Result<()>;
    async fn remove(&self, scope: McpCatalogScope, name: &str) -> Result<()>;
    async fn set_enabled(&self, scope: McpCatalogScope, name: &str, enabled: bool) -> Result<()>;
}
```

`McpCatalogEntry` is `ScopedMcpServer` plus the fields every host already keeps beside
it: `enabled`, `scope`, and a free-form `origin` string for provenance (a file path, a
row id, an extension name). Scope is an ordered host-defined list, not a fixed enum, so
yolop's `global | profile | workspace` and the product's `harness | agent | session`
both express precedence through the same merge without either being special-cased.

Implementations: `FileMcpCatalog` (shipped in `everruns-mcp` behind a `catalog-file`
feature, implementing the `.mcp.json` / `mcpServers` shape, `${VAR}` expansion, and
`normalize_server_entry_value`), which yolop adopts wholesale; and a DB-backed catalog
in `crates/server` for the hosted product.

### D3, `McpCapability` becomes the extensible management capability

A single capability with id `mcp`, constructed from an `Arc<dyn McpCatalog>`:

- Tool set is derived from what the catalog supports. A read-only catalog exposes only
  `list_mcp_servers`; a mutable one adds upsert/remove/enable. That is the
  extensibility point that lets the hosted product opt into agent-driven MCP management
  later without a second implementation.
- A `literal_credentials: bool` policy flag carries yolop's ACP rule (reject literal
  credential-bearing header/env fields when the prompt channel is not a secure input),
  which is a general property of the *channel*, not of yolop.
- `mcp_servers_with_config` returns the catalog's effective set, so the capability
  contributes its own servers through the seam that already exists in
  `everruns_core::capabilities::collect_capability_mcp_servers`. Hosts stop threading a
  separate config load into session construction.

This is the piece that "enables both": everruns registers it over the DB catalog,
yolop over the file catalog, and a third embedder over anything else.

### D4, Publish the host's discovery path

`crates/host/src/mcp.rs` gains a public surface (`pub mod mcp` under the existing `mcp`
feature) for `resolve_servers` / `endpoint_for` / `discover_tool_definitions`, or those
move down into `everruns-mcp` next to `McpServerCapability` with the cache staying in
the host. Either way yolop deletes `mcp_connection_for` and `discover_mcp_tool_names`
and gets the cache and bounded concurrency for `/tools` as a side effect. This is the
cheapest item on the list and the one that stops future field drift.

### D5, Move the env auth fallback upstream

`EnvMcpAuthProvider` becomes `everruns_mcp::auth::EnvAuthProvider`, and `ChainAuthProvider`
composes providers in order so `OAuth → env → none` is expressed once. yolop's
`StoredMcpAuthProvider` reduces to a construction site.

## Sequencing

Each step is independently shippable and independently useful downstream:

1. **D4 + D5** — pure deletion downstream, no new concepts, no config migration. Land
   first; it removes the drift hazard immediately.
2. **D1** — mechanical rename, internal-only.
3. **D2** — `McpCatalog` + `FileMcpCatalog`, with yolop's `McpConfigStore` reimplemented
   on top and its scope/merge tests moved up as the conformance suite.
4. **D3** — `McpCapability` over the catalog; yolop deletes `src/capabilities/mcp.rs`
   and registers the upstream one. `/mcp`, `yolop mcp`, and `RuntimeHandles::reload_mcp_servers`
   stay downstream (they are terminal and ACP concerns) but call the catalog.

## Non-goals

- Unifying the *storage formats* themselves. The hosted product keeps rows; yolop keeps
  files. The catalog seam exists precisely so they need not converge.
- Moving `/mcp`, `yolop mcp`, or the OAuth loopback browser flow upstream. Those are
  terminal-host concerns; the catalog and the capability are not.
- Changing tool naming, prefixing, or the execution path. Those are already shared and
  correct.

## Where it lives

| Concern | Location |
|---|---|
| Per-server virtual capability (D1) | `crates/mcp/src/capability.rs` |
| Catalog seam and file catalog (D2) | `crates/mcp/src/catalog.rs` (new) |
| Management capability (D3) | `crates/mcp/src/management.rs` (new) |
| Discovery/connection mapping (D4) | `crates/host/src/mcp.rs` |
| Auth providers (D5) | `crates/mcp/src/auth.rs` |
| Hosted DB catalog | `crates/server/src/domains/mcp_servers/` |
| Downstream adoption | yolop `src/config/mcp.rs`, `src/capabilities/mcp.rs`, `src/runtime/mod.rs` |
