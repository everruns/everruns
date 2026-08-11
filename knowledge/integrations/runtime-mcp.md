---
type: Specification
title: "Runtime MCP Client Specification"
description: "MCP client in the in-process runtime: shared `everruns-mcp` crate, transport abstraction (HTTP + optional stdio), pluggable auth."
tags:
  - everruns
  - integrations
---
# Runtime MCP Client Specification

> Part of the [MCP spec family](mcp.md). This document is the **decision record**
> for bringing the MCP (Model Context Protocol) **client** to the in-process
> [runtime](../foundations/runtime.md). It covers the shared client crate, transport
> abstraction (HTTP + optional stdio), pluggable authentication, and how the
> example coding CLI consumes MCP. Server/control-plane MCP-client behavior
> (org-managed servers, CRUD, gRPC) stays in [mcp-servers.md](mcp-servers.md);
> this spec extends that contract to embedded execution without duplicating it.

## Abstract

> **Status: implemented.** The `everruns-mcp` crate exists and the runtime
> builds `mcp_tool_definitions` from its scoped servers
> (`crates/host/src/mcp.rs`, `crates/host/src/runtime.rs`). The paragraph
> below records the original problem state that motivated the extraction.

The MCP client already works on the control plane: org-managed and scoped
`mcpServers` are discovered and executed by `everruns-worker` /
`everruns-server`. Originally the **in-process runtime could not reach any of
it**: the dependency direction is `core ← runtime ← worker ← server`, the live
MCP client (`call_mcp_tool`, `fetch_mcp_tools`, auth resolution) lived in
`worker`/`server`, and the runtime hardcoded `mcp_tool_definitions: vec![]`. The
example coding CLI embeds the runtime with no server, so it originally had no
MCP at all — even though the runtime builders already accepted
`.mcp_servers(ScopedMcpServers)` and plumbed it into `Harness`/`Agent`/`Session`.

This spec records the decision to **extract the transport-agnostic MCP client
into a new `everruns-mcp` crate** that `runtime`, `worker`, and `server` all
depend on, wire it into the runtime's discovery + execution path, add an
**optional stdio transport behind a cargo feature** (hard-off in hosted
builds), and make **credential acquisition pluggable** so non-web (CLI) hosts
can authenticate. Acceptance is MCP working in the example coding CLI.

## Motivation and constraints

The end goals for this work, and how the design meets each:

1. **Usable from the runtime.** Runtime embedders get MCP by configuring
   scoped `mcpServers` — no server, gRPC, or database required.
2. **Non-HTTP transport, hard-off in hosted capabilities.** stdio is supported
   for local runtime/CLI hosts but compiled out of the hosted product so it
   cannot exist there (multi-tenant process-spawn is out of scope for the
   hosted threat model).
3. **Reuse, not reinvention.** MCP remains a [virtual capability](../execution/capabilities.md);
   scoped-server merge, tool-name prefixing, SSRF validation, and content
   conversion are reused as-is.
4. **Session-style configuration.** The existing harness→agent→session
   `mcpServers` overlay (see [mcp-servers.md](mcp-servers.md)) is the only
   configuration surface; the runtime consumes it instead of ignoring it.
5. **Pluggable authentication.** The current OAuth flow is browser/redirect
   based. Runtime hosts are often CLI. Credential acquisition is an injectable
   trait so a CLI host can supply a static token / device-code flow while the
   server keeps its web OAuth.
6. **No duplication.** The worker/server stop carrying their own copy of the
   JSON-RPC client; they call `everruns-mcp`.
7. **`everruns-mcp` crate.** Adopted (goal 7's optional crate is the right
   seam here).
8. **Acceptance: coding CLI.** `examples/coding-cli` exposes MCP via config and
   a `/mcp` affordance; an integration test drives a tool call end to end.
9. **Integration tests.** The crate ships transport-level integration tests
   against a mock MCP server; the runtime and coding CLI ship end-to-end tests.

## Decisions

### D1 — New `everruns-mcp` crate

A new workspace crate `crates/mcp` (`everruns-mcp`) owns the transport-agnostic
MCP client. It depends only on `everruns-core` (for the wire types in
`mcp_server.rs`, `EgressService`, `validate_url_dns_pinned`, `ToolDefinition`,
`ToolResult`, and the `ToolExecutor`/`ToolContext` traits). It is depended on
by `runtime`, `worker`, and `server`.

What moves into `everruns-mcp` (deleted from `worker`/`server`):

| Today | Moves to |
|-------|----------|
| `worker/src/mcp_executor.rs::call_mcp_tool`, `extract_json_from_response`, MCP-content → `ToolResult` conversion | `everruns-mcp` transport + result mapping |
| `worker/src/mcp_executor.rs::{McpToolExecutor, CompositeToolExecutor}` | `everruns-mcp` executors (server resolution stays injectable) |
| `server/.../mcp_servers/service.rs::fetch_mcp_tools` (tools/list) | `everruns-mcp` discovery |

The wire types (`McpToolCallRequest`, `McpToolsListRequest`, `McpContent`, the
`McpError*` family, tool-name helpers) **stay in `everruns-core`** — they are
already shared by API/OpenAPI and moving them would churn many call sites for
no benefit (goal 6).

### D2 — Transport abstraction; stdio behind a cargo feature

Introduce a `McpTransport` trait in `everruns-mcp` with two methods
(`list_tools`, `call_tool`) that take a logical request and return parsed
JSON-RPC results. Two implementations:

- **`HttpTransport`** (default, always compiled): wraps the existing
  `EgressService` path — DNS-pinned SSRF validation
  (`validate_url_dns_pinned`), pinned-addr egress request, SSE-or-JSON
  response parsing. This is a straight lift of today's worker/server code, so
  hosted behavior is byte-for-byte unchanged.
- **`StdioTransport`** (behind `#[cfg(feature = "stdio")]`): spawns a local
  process and speaks newline-delimited JSON-RPC over stdin/stdout per the MCP
  stdio transport.

**Hard-off mechanism.** `everruns-mcp`'s `stdio` cargo feature is **not**
enabled by `server` or `worker`. The transport selector returns a typed
"transport not supported in this build" error for any non-HTTP `ScopedMcpServer`
when the feature is absent, and `McpServerTransportType` gains a `Stdio` variant
that the server's scoped-config validation rejects (mirroring the existing
`validate_scoped_mcp_servers` checks). The runtime and `examples/coding-cli`
enable the feature. This gives a compile-time guarantee that stdio code does
not exist in the hosted product, with a graceful runtime error as a
belt-and-suspenders secondary check. See [threat-model.md](../security/threat-model.md)
(new TM entry for runtime stdio MCP).

`ScopedMcpServer` gains the fields needed to describe a stdio server
(`command`, `args`, `env`), serialized to match the `.mcp.json`
local-server shape. These are ignored by HTTP transport and rejected by hosted
validation.

### D3 — Pluggable authentication (`McpAuthProvider`)

Credential acquisition becomes a trait in `everruns-mcp`:

```text
trait McpAuthProvider {
    // Resolve an Authorization value (or other headers) for a given
    // logical server + auth mode, acquiring/refreshing as needed.
    async fn authorization(&self, server: &McpAuthRequest) -> Result<Option<McpCredential>>;
}
```

- The **server** implements the trait over the existing session-secret +
  `UserConnectionResolver` web-OAuth resolution. Its resolver owns token
  lifetime: it lazily refreshes near-expiry grants, coalesces concurrent
  refreshes, and atomically persists refresh-token rotation before returning
  the new access token.
- The **runtime/coding-CLI** provides simpler implementations: a static
  bearer/header provider, an env-var provider, and `OAuthAuthProvider`.

The OAuth half is shared inside the MCP crate. `everruns_mcp::oauth::protocol`
(moved out of core in EVE-879) owns the
protocol steps — RFC 9728 protected-resource discovery, RFC 8414/OpenID
authorization-server metadata, RFC 7591 dynamic registration, PKCE, code
exchange, refresh, and RFC 9207 issuer validation — with no browser, no
listener, and no storage. `everruns_mcp::oauth` binds them to MCP: discovery
starts at the *server* (its metadata names the issuer; absent that, its origin
is the issuer), the token is bound to the server with a `resource` indicator
(RFC 8707), and `prepare_login`/`complete_login` split the flow so the host
supplies only the callback leg — a loopback listener for a CLI, a redirect
route for the control plane. Persistence is the `McpTokenStore` trait.

Every OAuth request goes through `EgressService` with DNS pinning. The
authorization-server URL is discovered *from the remote MCP server*, so it is
attacker-influenced input and must not bypass the egress boundary.
- `auth_mode = OAuth` no longer implies "web". "Lifetime"/refresh is owned by
  the provider implementation (e.g. a CLI provider may cache a token and
  refresh on 401), keeping the core executor stateless.

The existing untrusted-OAuth stripping for explicit scoped servers
(`scoped_mcp.rs::strip_untrusted_oauth_from_scoped_mcp_servers`) is preserved
and lives alongside the trait — explicit user config still cannot mint
connection tokens; only capability-contributed servers and the host-injected
auth provider can.

### D4 — Runtime wiring (discovery + execution)

Two integration points in `crates/host`:

1. **Discovery** — replace `mcp_tool_definitions: vec![]`
   (`runtime.rs:524`). The runtime resolves effective scoped servers from the
   harness→agent→session overlay (reusing `merge_scoped_mcp_servers`, already
   applied in `config_layer.rs`), runs `everruns-mcp` discovery for each server
   with `tool_discovery = true`, builds `McpCapability` tool definitions
   (existing `crates/core/src/capabilities/mcp.rs`), and feeds them into
   `ReasonInput.mcp_tool_definitions`. Discovery is **live** per turn (a
   `tools/list` per server), matching the control plane's scoped-server
   behavior, which keeps no persisted cache. A per-session TTL cache is a
   listed follow-up.
2. **Execution** — in `execute_act_activity` (`host.rs`), register the turn's
   MCP tools as first-class `Tool`s in the builtin `ToolRegistry` via
   `everruns_core::build_mcp_proxy_tools(&input.tool_definitions, invoker)`. Each
   `McpProxyTool` delegates execution to the host's `McpExecutor` (which
   implements `everruns_core::McpToolInvoker`). The turn's tool definitions
   already include the discovered MCP tools, so no re-discovery is needed, and
   because MCP tools live in the regular registry they are visible to everything
   that introspects it (`spawn_background`, `tool_search`, openai_tool_search
   namespaces, ...). The executor passed to `ActAtom` is just the registry.

Both points hang off an optional adapter hook
(`RuntimeHostAdapter::mcp_executor()` returning
`Option<Arc<everruns_mcp::McpExecutor>>`),
default `None`, so hosts that don't configure MCP pay nothing and behavior is
unchanged.

Capability-contributed MCP servers participate in the same runtime path. The
runtime dependency-resolves the effective capability set, collects its MCP
servers as defaults, then overlays explicit harness/agent/session servers by
logical name. A live capability change invalidates that session's discovery
cache, so activation discovers new servers on the next reason boundary and
deactivation cannot retain stale tool definitions. Stdio transport remains
per-invocation: each discovery/call tears down its child process, so removal
leaves no persistent process to disconnect.

> Historical note: an earlier design routed `mcp_*` calls through a separate
> `CompositeToolExecutor` wrapper instead of registering MCP tools in the
> registry. That kept MCP tools invisible to registry-introspecting tools and
> has been replaced by the registry-proxy model above.

### D5 — Reuse in worker/server (no duplication)

`worker`/`server` switch their MCP call sites to `everruns-mcp`:

- The worker supplies gRPC-backed server resolution (a `McpConnectionResolver`
  over `get_mcp_server_by_prefix`) and builds an `everruns-mcp` `McpExecutor`
  from its `mcp_executor()` hook; MCP tools then register into the registry like
  any other host. Resolved credentials are baked into the connection headers, so
  the shared client needs no auth provider.
- `build_scoped_mcp_tool_definitions` calls `everruns-mcp` discovery instead of
  the local `fetch_mcp_tools`.

The existing worker/server tests (SSRF blocks, SSE parsing, image extraction,
tool-name round-trips) move with the code or are re-pointed, so the safety net
is retained.

## Configuration

Configuration is **only** the existing scoped `mcpServers` overlay from
[mcp-servers.md](mcp-servers.md) — no new top-level surface (goal 4). Runtime
embedders use the builder API that already exists
(`HarnessBuilder`/`AgentBuilder`/`SessionBuilder::mcp_servers`,
`crates/host/src/builders.rs`). Example, HTTP:

```rust
SessionBuilder::default().mcp_servers(serde_json::from_value(json!({
    "docs": { "type": "http", "url": "https://example.com/mcp" }
}))?)
```

Example, stdio (runtime/CLI builds only):

```json
{ "mcpServers": { "fs": { "type": "stdio", "command": "mcp-server-filesystem", "args": ["/work"] } } }
```

The coding CLI additionally reads a `.mcp.json` from the workspace root (the
same shape) so users configure MCP the way every other MCP client expects.

## Acceptance: example coding CLI

`examples/coding-cli` (`everruns-coding-cli`):

- Enables `everruns-mcp`'s `stdio` feature.
- Loads `mcpServers` from workspace `.mcp.json` (and/or a `--mcp` flag) and
  passes them to the single-session builder.
- Injects a static/env `McpAuthProvider` for authenticated servers.
- Adds a `/mcp` slash command listing configured MCP servers.
- One-shot smoke: `ercode --print "use the docs MCP tool to ..."` resolves and
  calls an MCP tool.

## Testing strategy (goal 9)

- **`everruns-mcp` unit/integration**: against a `wiremock` HTTP MCP server —
  `tools/list`, `tools/call`, SSE vs plain JSON, image extraction, error
  mapping, and SSRF blocks (localhost/private/metadata/IPv6). With `stdio`
  enabled, a fixture echo MCP process exercises spawn/list/call/teardown.
- **Auth**: a fake `McpAuthProvider` asserts the resolved header reaches the
  transport; the web-OAuth adapter retains its session-secret/connection tests.
- **Runtime**: an in-process runtime + mock MCP server asserts a turn discovers
  an MCP tool, the LLM-sim calls it, and the result returns — closing the
  `vec![]` gap with a regression test.
- **Coding CLI**: `--print` end-to-end against a mock MCP server (HTTP) and a
  fixture stdio server.

## Security considerations

- HTTP transport keeps the existing **DNS-pinned SSRF** contract
  (TM-TOOL-018); no relaxation.
- **stdio is excluded from hosted builds at compile time** and rejected by
  hosted scoped-config validation; it executes arbitrary local processes and is
  only acceptable for single-tenant runtime/CLI hosts. New threat-model entry
  documents the boundary.
- Explicit (user-supplied) scoped OAuth servers remain stripped of token
  minting; only capability-contributed servers and the host-injected auth
  provider authenticate.
- Pluggable auth must not weaken the above: providers receive only the logical
  server identity and never the runtime's connection-resolver internals.

## Dismissed alternatives

- **Put the client in `everruns-core`.** Rejected: pulls HTTP (and, with
  stdio, process-spawn) deps into the crate everything depends on.
- **Runtime depends on `everruns-worker`.** Rejected: inverts the existing
  `worker → runtime` direction and creates a cycle.
- **Gate stdio with a runtime env flag only.** Rejected as the *primary*
  mechanism: ships the code into hosted binaries. Kept only as a secondary
  runtime check; the compile-time cargo feature is the hard boundary.
- **Auth lifetime as a TTL config knob.** Superseded: the real requirement is
  pluggable acquisition for non-web hosts; lifetime/refresh is a provider
  implementation detail.

## Implementation map

| Concern | Location |
|---------|----------|
| New crate | `crates/mcp` (`everruns-mcp`) |
| Transport trait + HTTP impl | `everruns-mcp` (lift from `worker/src/mcp_executor.rs`, `server/.../service.rs::fetch_mcp_tools`) |
| stdio transport | `everruns-mcp`, `#[cfg(feature = "stdio")]` |
| Auth provider trait | `everruns-mcp`; web-OAuth adapter in `server`/`worker` |
| Scoped types (`command`/`args`/`env`, `Stdio` variant) | `crates/core/src/mcp_server.rs` |
| Runtime discovery | `crates/host/src/runtime.rs` (replace `vec![]`), `crates/host/src/mcp.rs` (live per-turn discovery) |
| Runtime execution | `crates/host/src/host.rs::execute_act_activity` (composite executor) |
| Adapter hook | `RuntimeHostAdapter::mcp_executor()` |
| Coding CLI | `examples/coding-cli` (`.mcp.json`, `/mcp`, auth provider) |

## Implementation status

Landed:

- `everruns-mcp` crate: `McpTransport` (HTTP always-on; stdio behind `stdio`),
  `McpAuthProvider`, `McpClient`, `McpExecutor`/`CompositeToolExecutor`, and
  free `http_*` functions for `&dyn EgressService` callers.
- Runtime (D4): `InProcessRuntime` discovers scoped MCP tools in
  `load_resolved_turn` and routes `mcp_*` calls via the `mcp_executor()` adapter
  hook (default `None` → unchanged for the worker). Builder
  `mcp_auth_provider()`; off-by-default `mcp-stdio` feature.
- stdio scoped config (D2): `ScopedMcpServer` gained `command`/`args`/`env` and
  `McpServerTransportType::Stdio`; the runtime maps stdio servers under
  `mcp-stdio`, and the hosted server rejects stdio in scoped-config validation
  and org MCP-server create/update.
- Dedup (D5/goal 6): the worker's local JSON-RPC client was removed (only
  `McpServerInfo` remains); the control plane's `fetch_mcp_tools` delegates to
  `everruns-mcp::http_list_tools`.
- Coding CLI (D8): reads `.mcp.json` (HTTP + stdio), wires servers into the
  session, adds a `/mcp` command.
- Multi-era protocol negotiation (D9): the HTTP transport speaks legacy
  (`2025-03-26`), current (`2025-06-18`), and the 2026 stateless RC
  (`2026-07-28`) through one code path. `McpConnection.protocol_mode` (from
  `McpServer`/`ScopedMcpServer`/`McpServerInfo`) selects the policy; `auto`
  tries stateless-first and falls back to the `initialize` handshake +
  `Mcp-Session-Id`, caching the verdict per server. Every request carries
  `_meta` (client info) and routable headers (`MCP-Protocol-Version`,
  `Mcp-Method`, `Mcp-Name`). Pure pieces in `crates/mcp/src/protocol.rs`,
  orchestration in `http.rs`. Contract detail in
  [mcp-servers.md](mcp-servers.md) ("Multi-era protocol support").

Remaining follow-up:

- Wire hosted (worker) remote-MCP **execution** through `everruns-mcp` via a
  gRPC-backed `McpConnectionResolver` + web-OAuth `McpAuthProvider`. Discovery
  is shared today; execution still needs this adapter (the worker's old
  executor was already dead code).
- Per-session TTL cache for runtime tool discovery (currently live per turn,
  matching the control plane's scoped-server behavior).
