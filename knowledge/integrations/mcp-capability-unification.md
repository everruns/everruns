---
type: Specification
title: "MCP Capability Unification"
description: "One MCP catalog and one command declaration shared by the hosted product and embedding hosts such as yolop, plus the discovery and auth code they duplicate."
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

The decision: introduce one **MCP catalog trait** in `everruns-mcp`, so a host supplies
*where servers are stored* and inherits identity, discovery, enablement and reload for
free, and declare the management operations **once as commands** rather than as model
tools, because both products have now independently settled on a CLI grammar for
administration.

## Problem state

### The word "capability" names two different things

| Thing | Where | What it is | ID |
|---|---|---|---|
| `everruns_mcp::McpCapability` | `crates/mcp/src/capability.rs` | A **virtual capability per server**: wraps one server's cached `tools/list` result and emits prefixed `ToolDefinition`s with capability attribution. `tools()` returns empty; execution goes through `McpExecutor`. | `mcp:<uuid>` |
| yolop `McpCapability` | yolop `src/capabilities/mcp.rs` | A **management capability**: four agent-facing tools (`list_mcp_servers`, `upsert_mcp_server`, `remove_mcp_server`, `set_mcp_server_enabled`) over a file-backed config store. Contributes no MCP tools. | `mcp` |

They collide on name to the point that yolop's runtime imports its own as
`YolopMcpCapability`. Neither is a superset of the other. yolop has no per-server
virtual capability (it never registers one; it only borrows
`McpCapability::tool_definitions` indirectly through the host), and the hosted
product exposes no *management capability* either: its CRUD lives in
`crates/server/src/domains/mcp_servers/` behind REST, gRPC, and the
`everruns mcp-servers <verb>` command tree, never as model tools.

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
| Mutation surface | REST, gRPC, UI, `everruns mcp-servers` command tree | agent tools, `/mcp` command, `yolop mcp` CLI |
| Live reload | per-turn resolution from the DB | `RuntimeHandles::reload_mcp_servers` swapping `session.mcp_servers` |

Two of these are *general* and only accidentally live downstream:
`normalize_server_entry_value` (every `.mcp.json` in the ecosystem uses that shape) and
the per-entry `enabled` flag (the hosted product expresses the same idea with archived
rows).

### Both products already unified administration, separately

The larger convergence happened while this analysis was being written, in both
repositories at once, and neither knows about the other's version:

| | everruns | yolop |
|---|---|---|
| Contribution unit | `Command::cli() -> Option<CliRoute>` on a domain command | `ControlRoute` on a `Capability` |
| Registry | `CliCommandSource::specs()` | `CliRegistry` of `CliCapability` |
| Grammar | declared noun path plus verb | a clap `Command` the capability builds |
| Root token | `CliCommandSource::root()`, default `everruns` | the `yolop` binary |
| Transport | in-process bashkit builtin, or rewrite for `ScriptedTool` | spawn own executable, one-shot pipes to the parent session |
| Read-only marking | per-command `policy()` | `ControlRoute::read_only_operations` |
| Prompt cost | scripted tool description plus a catalog `cli` field | one shared block, each route contributes one clause |

Upstream this is [Command Tree](../execution/command-tree.md), with `mcp-servers` in
the first tranche and the root token made a host's own concern (#3496). In yolop it is
`src/control.rs`, and MCP finished the move in #685: `McpCapability::tools()` now
returns an empty vector, mutation lives entirely in `yolop mcp ...` and `/mcp ...` with
automatic live reload, and only the read-only `list` is served inline on the control
route.

So MCP management as model tools is gone from both products. What remains duplicated is
the *machinery*, twice, and the catalog underneath it, twice.

### Host-private code forces downstream duplication

`crates/host/src/mcp.rs` is `mod mcp;`, not `pub mod`. Everything in it is
`pub(crate)`. Consequently yolop carries byte-level copies:

- `src/runtime/mod.rs::mcp_connection_for` duplicates `host::mcp::endpoint_for` +
  `resolve_servers` (transport match, `McpConnection` construction, empty
  `secret_bindings`).
- `src/runtime/mod.rs::discover_mcp_tool_names` duplicates
  `host::mcp::discover_tool_definitions` minus the cache and concurrency, so `/tools`
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

### D2, One `McpCatalog` trait owning storage

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

### D3, One MCP command declaration over the catalog, not a tool set

The original version of this decision proposed a management capability carrying four
model tools. Both products have since rejected that shape, so it is withdrawn.

What is shared instead is a **command declaration**: `list`, `add`, `remove`, `enable`,
`disable`, `login`, declared once against `McpCatalog` with their argument shapes,
read-only marking, and examples, and rendered by whichever CLI machinery the host
already has. The capability still exists, because servers reach a session through
`Capability::mcp_servers_with_config` and `collect_capability_mcp_servers`, but it
contributes servers and a command declaration, not tools.

Two properties carry over from the withdrawn version because they are real:

- Mutation is optional. A read-only catalog (the hosted product's, until it chooses
  otherwise) declares only `list`, and no host renders a write command it cannot serve.
- A `literal_credentials` policy belongs to the *channel*, not to yolop: rejecting
  literal credential-bearing header and environment fields is correct wherever the
  input path is not a secure one, which is why yolop applies it under ACP.

### D6, Move the command-tree contract out of the bashkit integration

`CliRoute`, `CliCommandSpec`, `CliCommandSource`, `CliTree`, the help renderer, and the
statement rewriter live in `integrations/bashkit/src/cli.rs`. Only two lines of that
1269-line file touch bashkit: one `use bashkit::ExecResult` and one
`impl bashkit::Builtin` block at the end. yolop has no bashkit dependency at all (it
runs real processes through `src/exec/`), so today it cannot adopt any of it.

Splitting the grammar, tree, source trait and help rendering into a neutral crate, and
leaving the builtin adapter behind in the integration, would let yolop's `CliCapability`
become a `CliCommandSource` implementation and yolop's binary the tree's root token,
which #3496 already made a host's choice. That is the change that actually makes one
administration grammar serve both products, and it is independent of MCP: MCP is simply
the first domain that would cross it.

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

1. **D4 + D5**: pure deletion downstream, no new concepts, no config migration. Land
   first; it removes the drift hazard immediately and needs no agreement about
   administration at all.
2. **D1**: mechanical rename, internal-only.
3. **D6**: extract the command-tree contract from the bashkit integration. Independent
   of MCP, and the prerequisite for D3 being one declaration rather than two.
4. **D2**: `McpCatalog` plus `FileMcpCatalog`, with yolop's `McpConfigStore`
   reimplemented on top and its scope and merge tests moved up as the conformance suite.
5. **D3**: one MCP command declaration over the catalog, rendered by both trees. yolop's
   `/mcp`, `yolop mcp`, and `RuntimeHandles::reload_mcp_servers` stay downstream (they
   are terminal and ACP concerns) but read and write through the catalog.

D4, D5 and D6 are each worth landing on their own merits even if the catalog work never
follows.

## Non-goals

- Unifying the *storage formats* themselves. The hosted product keeps rows; yolop keeps
  files. `McpCatalog` exists so they need not converge.
- Moving `/mcp`, the terminal rendering, or the OAuth loopback browser flow upstream.
  Those are terminal-host concerns; the catalog and the command declaration are not.
- Making yolop depend on bashkit. D6 moves a contract to neutral ground rather than
  moving a shell into a host that already has one.
- Changing tool naming, prefixing, or the execution path. Those are already shared and
  correct.

## Where it lives

| Concern | Location |
|---|---|
| Per-server virtual capability (D1) | `crates/mcp/src/capability.rs` |
| Catalog trait and file catalog (D2) | `crates/mcp/src/catalog.rs` (new) |
| MCP command declaration (D3) | `crates/mcp/src/commands.rs` (new) |
| Command-tree contract (D6) | `integrations/bashkit/src/cli.rs`, moving to a neutral crate |
| Discovery/connection mapping (D4) | `crates/host/src/mcp.rs` |
| Auth providers (D5) | `crates/mcp/src/auth.rs` |
| Hosted DB catalog | `crates/server/src/domains/mcp_servers/` |
| Downstream adoption | yolop `src/config/mcp.rs`, `src/capabilities/mcp.rs`, `src/runtime/mod.rs` |
