---
type: Specification
title: "MCP Resources: reading attached servers' resources"
description: "Adoption plan for MCP resources (spec 2025-11-25): agents list and read resources on their attached MCP servers, tool results keep every content block, and the /mcp endpoint catches up on templates, metadata and errors."
tags:
  - everruns
  - mcp
  - integrations
---
# MCP Resources: reading attached servers' resources

Status: steps 1 to 4 implemented (2026-10-11); step 5 not started. Spec: [MCP 2025-11-25, server resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources).

## Why

Many MCP servers publish their most useful context as resources, not tools:
files, schemas, tickets, documents. Today an agent attached to such a server
sees only its tools, so that context is out of reach. Resources are also how
tool results point at bigger payloads (`resource_link`, embedded `resource`),
and those blocks do not survive our client today.

## Where we are

| Area | State | Source |
|---|---|---|
| Client: `resources/list`, `resources/templates/list`, `resources/read` | Not implemented. The client speaks `tools/list` and `tools/call` only, and does not keep the server's `resources` capability from `initialize`. | [`crates/core/src/mcp/client.rs`](../../crates/core/src/mcp/client.rs), [`protocol.rs`](../../crates/core/src/mcp/protocol.rs) |
| Client: content blocks in tool results | Out of spec. `McpContent` reads `image` with a `mime_type` key (the wire key is `mimeType`), reads embedded `resource` flat (the wire nests it under `resource` and allows `blob`), and has no `audio` or `resource_link`. Serde rejects the whole `tools/call` result when any block does not match, so a spec-compliant image, embedded resource or link fails the call. Read from the code; step 1 starts with the failing test. | [`McpContent`](../../crates/contracts/src/runtime/mcp_server.rs), [`result.rs`](../../crates/core/src/mcp/result.rs) |
| Client: subscriptions, `list_changed`, `updated` notifications | Not implemented. | |
| Server (`/mcp`): `resources/list`, `resources/read` | Implemented: four org-scoped JSON catalogs (`everruns://agents`, `capabilities`, `harnesses`, `models`) and MCP Apps `ui://` templates, with `2026-07-28` cache hints. | [`mcp_endpoint/resources.rs`](../../crates/server/src/api/mcp_endpoint/resources.rs), [mcp.md](mcp.md) |
| Server: templates, metadata, errors | Gaps: no `resources/templates/list`; entries have no `title` or `annotations`; an unknown URI answers `-32602` instead of `-32002` with `data.uri`; `cursor` is ignored (fine while the list is one page). | same |
| Protocol version | Neither side lists `2025-11-25`; client and server negotiate `2026-07-28`, `2025-06-18`, `2025-03-26`. | [`policy.rs`](../../crates/contracts/src/runtime/mcp_server/policy.rs), [`mcp_endpoint/mod.rs`](../../crates/server/src/api/mcp_endpoint/mod.rs) |

## Decisions

### D1. Agents read resources through two shared tools

When the turn has at least one MCP server attached, deferred servers
included, the agent gets two tools:

- `list_mcp_resources { server, cursor? }` returns the server's resources
  (`uri`, `name`, `title`, `description`, `mimeType`, `size`, `annotations`),
  its resource templates on the first page, and the server's `nextCursor`.
- `read_mcp_resource { server, uri }` returns the contents.

Two shared tools, not two per server: the tool list stays flat however many
servers are attached. The names start with the verb so they never look like a
server tool (`mcp_<server>__<tool>`) or a deferred server placeholder
(`mcp_<server>`), which the runtime parses by the `mcp_` prefix. A URI from a
template is filled by the model and read with `read_mcp_resource`, which is
all the spec asks of a client. Completion (`completion/complete`) is out of
scope. `server` is required: one call reaches one server, and only servers
attached to the turn are reachable.

The tools are offered on any MCP server, not only those that declared
`resources`: stateless HTTP servers are not initialized before the turn, so
the capability is not known when the tool list is built. A server without
resources answers `-32601`, which the tool reports as "does not offer
resources". No feature flag: attaching a server is the opt-in.

Resources are read with the same credential and `actsAs` as that server's
tools, and the same egress rules apply. Both tools are rated as read-only
in the tool risk labels.

### D2. Content mapping

Read contents and tool-result blocks map the same way:

- `text` contents are returned inline, cut by the usual tool-output
  truncation, with the cut stated.
- `blob` with an `image/*` MIME type becomes a native image block, like an
  image tool result today.
- Any other `blob` is never inlined as base64. In a tool result it is
  described (`uri`, `mimeType`, size). `read_mcp_resource` writes it to the
  session workspace when the harness has one and returns the path.
- `resource_link` in a tool result is passed to the model as a link
  (`uri`, `name`, `mimeType`, `description`), readable with
  `read_mcp_resource` against the same server.
- Content annotated `audience: ["user"]` is kept out of the model's
  context; `priority` and `lastModified` pass through as metadata.
- Unknown block types are kept as a short placeholder instead of failing the
  call.

### D3. No subscriptions in this round

`resources/subscribe` and `notifications/resources/updated` need a standing
server-to-client stream that outlives a turn. Our client is request and
response per turn, and agents that need push already have
[MCP Event triggers](mcp-events.md). The client does not ask to subscribe and
does not cache resource lists across turns, so `list_changed` has nothing to
invalidate. Revisit when a server we care about publishes resources that
change while a session waits.

### D4. Chat attach picker is a later, separate step

The spec's user interaction model is application-driven: the host lets the
person pick a resource and adds it to context. A Chat "attach from MCP"
picker built on the same list and read calls fits that, but it is UI work on
its own and is listed last.

### D5. `/mcp` server catches up on the same revision

The endpoint keeps its catalogs and adds what 2025-11-25 defines:
`title` and `annotations` (`audience: ["assistant"]`, `lastModified` where
the catalog has one) on each entry; `resources/templates/list` with
`everruns://agents/{agent_id}` and `everruns://sessions/{session_id}`, read
through the same org-scoped commands as the matching tools; `-32002` with
`data.uri` for an unknown URI; a `cursor` parameter accepted and no
`nextCursor` while the catalog is one page. The capability stays
`resources: {}` (no `subscribe`, no `listChanged`).

### D6. Negotiate `2025-11-25`

Both client and server add `2025-11-25` to their supported versions, between
`2026-07-28` and `2025-06-18`, so servers that stop at the current stable
revision negotiate it instead of falling back to `2025-06-18`.

## Plan

One PR each, merged when green.

1. **Content blocks per spec.** Failing test first with spec-shaped `image`,
   `audio`, embedded `resource` (text and blob) and `resource_link` results;
   fix `McpContent` and `map_tool_call_result`, tolerate unknown types.
2. **Client methods.** `list_resources` (paginated), `list_resource_templates`,
   `read_resource` on the HTTP and stdio transports and `McpClient`; keep the
   server's `resources` capability from `initialize`; add `2025-11-25` to the
   client's versions.
3. **Agent tools.** `list_mcp_resources` and `read_mcp_resource` on the
   hosted worker and in-process paths, D2 mapping, risk labels, tool narration,
   docs page; integration test against a local MCP server, and an example run
   against a real model.
4. **`/mcp` server.** D5 and the server half of D6, with endpoint tests and
   the [mcp.md](mcp.md) update.
5. **Chat attach picker** (D4), only if wanted after 1 to 4.

## Out of scope

Subscriptions and `list_changed` (D3), argument completion, resource
templates on the agent tool surface beyond listing them, and yolop (it
consumes the core client, so it gains the client methods with its next
`everruns` bump).
