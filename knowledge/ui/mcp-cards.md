---
type: Specification
title: "MCP Entity Cards"
description: "MCP Apps entity cards and sandboxed HTML resources."
tags:
  - everruns
  - ui
---
# MCP Entity Cards

## Abstract

The Everruns MCP endpoint (`/mcp`, see [`knowledge/integrations/mcp.md`](../integrations/mcp.md)) exposes
*entity cards*, small interactive HTML views of first-party Everruns entities
(Agent first, then Session, Harness, App, Capability, …). Cards are returned as
MCP **embedded resources** with the [`ui://`] URI scheme so any MCP-Apps-aware
host (Claude Desktop, mcp-ui clients, the Everruns chat UI itself) can render
them in a sandboxed iframe alongside textual tool output.

Cards are read-only in the first iteration but are designed end-to-end so that
later iterations can wire **mutation actions** (run agent, archive, edit) into
the card without protocol changes, actions flow back to the host via
`window.postMessage` and are dispatched as ordinary MCP `tools/call`
invocations on the same Everruns server.

This spec is **scoped to the Everruns MCP server endpoint**. It does not apply
to the REST/UI APIs and does not change how Everruns acts as an MCP *client*
to remote MCP servers (covered in [`knowledge/integrations/mcp-servers.md`](../integrations/mcp-servers.md)).

## Goals

- Standard, uniform UI surface for Everruns entities exposed over MCP.
- Works in any host that implements the embedded-resource + `ui://` pattern
  (no Everruns-specific host code required for read-only cards).
- Strict sandboxing: no network, no same-origin access, no agent-controlled
  HTML escapes.
- Forward-compatible with mutation actions through a stable `postMessage`
  contract.
- One Rust card module shared across entity types.

## Non-Goals

- Replacing the JSON tool/resource surface. Every card has a JSON sibling
  (`get_agent`, `everruns://agents`, etc.); the card never adds information
  that is not already exposed in JSON.
- Targeting hosts that cannot render `text/html` resources. Such hosts fall
  back to the existing JSON tools.
- Reusing OpenUI / A2UI for cards. Those frameworks are scoped to LLM-authored
  generative UI inside chat messages; cards are server-authored UI returned
  from MCP and need a different trust model and a different audience (any MCP
  host, not only the Everruns chat UI).

## URI Scheme

All cards use the `ui://` URI scheme prescribed by the MCP-UI / MCP Apps
convention:

```
ui://everruns/{entity}/{public_id}/card
```

- `entity` is the entity kind in singular, kebab-case: `agent`, `session`,
  `harness`, `app`, `capability`.
- `public_id` is the entity's external prefixed ID (e.g. `agent_01933b…`).
  See [`knowledge/foundations/id-schema.md`](../foundations/id-schema.md).
- The `/card` suffix leaves room for future view variants
  (`/card/compact`, `/timeline`, …) without breaking existing URIs.

The `ui://` URIs are **opaque identifiers**, not fetchable URLs. They are
returned inline as embedded resources from `tools/call` (and, in a follow-up,
from `resources/read`). Hosts MUST NOT attempt to dereference them over HTTP.

## MCP Wire Format

A card is returned as a single embedded resource inside a tool result:

```json
{
  "content": [
    {
      "type": "resource",
      "resource": {
        "uri": "ui://everruns/agent/agent_01933b5a000070008000000000000001/card",
        "mimeType": "text/html",
        "text": "<!doctype html>…"
      }
    },
    {
      "type": "text",
      "text": "Agent: customer-support — 12 sessions, 1.2M tokens used."
    }
  ]
}
```

- `type: "resource"` with a `resource.text` payload follows the MCP content
  type table in [`knowledge/integrations/mcp.md#content-types`](../integrations/mcp.md#content-types).
- The trailing `text` content block is a short plaintext summary so hosts
  that ignore the embedded resource still receive useful output.
- Card tools run only under negotiated protocol version `2025-06-18`. Under
  `2025-03-26` the card tool is omitted from `tools/list` and rejected from
  `tools/call`; clients should fall back to the JSON `get_agent` /
  `list_agents` tools.

### Card Tool Naming

Per-entity card tools use the convention `<entity>_get_card`, e.g.
`agent_get_card`. Each accepts the entity's public ID and an optional
`organization_id` override (same shape as other org-scoped tools, see
[`knowledge/integrations/mcp.md#per-call-organization_id-override`](../integrations/mcp.md#per-call-organization_id-override)).

Tools are registered alongside the existing tier-1 set in
`crates/server/src/api/mcp_endpoint/tool_registry.rs`. Annotations:
`read_only_hint: true`, `destructive_hint: false`, `idempotent_hint: true`,
`open_world_hint: false`.

### Resource Templates (deferred)

`resources/templates/list` exposure of card URIs as templated resources
(`ui://everruns/agent/{agent_id}/card`) is planned but not part of the first
iteration, see *Open Questions* below.

## HTML Contract

Cards are self-contained HTML documents:

- Single document, `<!doctype html>` + `<meta charset="utf-8">`.
- All styling inline or in a single `<style>` block. No external stylesheets.
- No external scripts. The only allowed `<script>` is one inline block
  containing the action wiring (see *Action Protocol*). A card with no
  actions has no scripts at all.
- No `<iframe>`, no `<object>`, no `<embed>`, no `<form action>`.
- Total document size MUST NOT exceed **64 KiB** after server-side
  rendering. The renderer rejects oversized cards rather than truncating.

All entity-controlled fields (`name`, `display_name`, `description`, tag
strings, capability IDs, …) are HTML-escaped. The card module exposes one
escape primitive and refuses to interpolate raw `Value`s.

### Visual Style

Cards are intentionally minimal:

- Neutral palette (system / dark-mode-aware via `prefers-color-scheme`).
- One header row with display name + status badge.
- A description block (raw text, not Markdown, link-rendering is the host's
  job for now).
- A stats grid: cumulative token totals, session count, last-activity date.
- A future `<div data-actions>` slot for buttons (see *Action Protocol*).

The first version pins the card at ~360 px wide so it composes inside chat
panes; hosts may resize the iframe.

## Action Protocol (Phase 2, design now, ship later)

When mutation actions are added, buttons inside the card communicate with the
host via `window.parent.postMessage(payload, "*")`. The payload schema is
borrowed from the [mcp-ui](https://github.com/idosal/mcp-ui) convention so
third-party MCP hosts can render Everruns cards without Everruns-specific
code.

```ts
type CardMessage =
  | { type: "tool";   payload: { toolName: string; params: Record<string, unknown> } }
  | { type: "prompt"; payload: { prompt: string } }
  | { type: "intent"; payload: { intent: string; params?: Record<string, unknown> } }
  | { type: "link";   payload: { url: string } }
  | { type: "notify"; payload: { message: string } };
```

Routing:

- `tool` → host invokes `tools/call` against the same MCP server. The host
  MUST surface a confirmation step for any tool whose annotations declare
  `destructive_hint: true` or `read_only_hint: false`.
- `intent` → host-defined navigation hook (e.g. `open_agent`). Hosts that
  don't know the intent ignore it.
- `link` → host opens the URL with `noopener`.
- `prompt` → host injects the string into the active chat as if typed.
- `notify` → host shows a toast / log line.

Cards never call `tools/call` themselves: they only request the host to.
The host is the trust boundary, not the iframe.

## Sandboxing Requirements (Host)

Hosts that render `ui://everruns/...` resources MUST:

1. Render the HTML inside an `<iframe>` with `sandbox="allow-scripts"`,
   crucially **without** `allow-same-origin`, `allow-top-navigation`,
   `allow-forms`, or `allow-popups`.
2. Set `referrerpolicy="no-referrer"` on the iframe.
3. Honour the strict Content-Security-Policy embedded in the document
   (the Everruns MCP endpoint emits one inline in `<meta http-equiv>`):
   `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'none'`.
   Note the policy is delivered as a `<meta>` tag inside `srcdoc`
   content, not an HTTP response header, `frame-ancestors` and a few
   other directives are not enforceable in this delivery mode, so iframe
   sandboxing is the primary clickjacking and same-origin defense.
4. Never fetch the HTML over HTTP via the iframe `src` attribute,
   populate the iframe from the resource `text` field directly via
   `srcdoc`. (`srcdoc` keeps the document inline so no network request
   is made and the document inherits the iframe's opaque sandboxed
   origin.)
5. Validate inbound `postMessage` events against the iframe's
   `MessageEvent.source` and ignore messages from any other window.
6. Apply the existing user-permission model to any `tool` action requested
   by a card. Cards do not bypass policy checks.
7. Rate-limit messages from a single iframe (suggested: 10 messages per
   second; drop on exceed).

The Everruns chat UI implements these requirements in
`apps/ui/src/components/mcp/mcp-card-iframe.tsx`. Chat transcript tool
activity surfaces render any successful `ui://` + `text/html` MCP resource
inside this iframe, including resources wrapped in the remote-MCP executor's
`{"content":[...]}` JSON text envelope. The text content from that envelope is
used as the normal details/fallback output. A standalone showcase lives under
`apps/ui/src/app/dev/mcp-cards/`.

## Server Module

Implementation lives in `crates/server/src/api/mcp_endpoint/cards.rs`:

- `EntityCard` struct (entity kind, public ID, title, optional subtitle
  and description, status, tags, stats, footer lines).
- `render_html(card: &EntityCard) -> Option<String>`, emits the
  sandboxed document with the CSP meta tag. Returns `None` when the
  rendered output exceeds `MAX_CARD_BYTES`; callers surface this as a
  tool error rather than truncating.
- `card_uri(kind: EntityKind, public_id: &str) -> String`.
- `card_tool_content(card: &EntityCard, summary: &str) -> Result<Value, String>`
, produces the `tools/call` content array (resource + summary text).
- HTML escape helper (`escape_html`) used by every interpolation point.
- `MAX_CARD_BYTES = 64 * 1024` enforced before returning.

Each entity adds a small adapter that maps the domain `Command` output into
an `EntityCard`. The first adapter is `agent_card(agent: &Agent, stats: AgentCardStats) -> EntityCard`,
which uses the existing `GetAgent` command plus a new aggregate read for
session count. See
`crates/server/src/storage/backend.rs::count_sessions_for_agent` for the
session counter.

## Stats Surface

Initial Agent card stats:

- Cumulative `TokenUsage` (`input_tokens`, `output_tokens`,
  `cache_read_tokens`, `cache_creation_tokens`), already on
  `Agent::usage`.
- Session count via `count_sessions_for_agent`.
- `created_at`, `updated_at`, `archived_at` (when archived).

Future entities (Session, Harness, App) reuse `SessionAggregateStatsRow`
from `crates/server/src/storage/models.rs`.

## Threats and Mitigations

See [`knowledge/security/threat-model.md`](../security/threat-model.md) section *7b. MCP Server
(TM-MCP)* for the canonical entries. New entries introduced by cards:

- **TM-MCP-003** Card HTML XSS via entity-controlled fields. *Mitigated* by
  the central `escape_html` helper applied to every interpolation, the
  inline CSP meta tag, and host-side iframe sandboxing without
  `allow-same-origin`.
- **TM-MCP-004** Card-driven CSRF or unauthorized state change. *Mitigated*
  by routing all card actions through host-side `tools/call`, which
  re-applies normal MCP auth + policy + per-call `organization_id`
  resolution. Cards have no out-of-band write path.
- **TM-MCP-005** Card-induced denial of service via oversized HTML.
  *Mitigated* by the `MAX_CARD_BYTES` cap on render.

## Future Entities

Each future card follows the same recipe and adds at most one entry per row
to this table:

| Tool                    | URI scheme                                   | First-iteration stats |
|-------------------------|----------------------------------------------|-----------------------|
| `agent_get_card`        | `ui://everruns/agent/{agent_id}/card`        | tokens, session count, last-activity |
| `session_get_card`      | `ui://everruns/session/{session_id}/card`    | duration, status, token usage |
| `harness_get_card`      | `ui://everruns/harness/{harness_id}/card`    | session count, capability count |
| `app_get_card`          | `ui://everruns/app/{app_id}/card`            | invocation count, last invocation |
| `capability_get_card`   | `ui://everruns/capability/{capability_id}/card` | usage count, status |

Only `agent_get_card` ships in the first iteration.

## Plugin Surface

The `everruns-dev` plugin exposes a slash command for the new tool:

- `commands/agent-card.md` → calls `agent_get_card` with a positional
  agent ID or name (resolved via `query` if not a valid `agent_…` ID).

The skill (`plugins/everruns-dev/skills/everruns-dev/SKILL.md`) documents
the `ui://` resource URI shape so coding agents know to surface the card
resource alongside textual replies in MCP-UI-aware hosts, and to fall back
to `get_agent` JSON otherwise.

## Open Questions

- Whether to expose templated card URIs through `resources/templates/list`
  in addition to per-entity `_get_card` tools. This unlocks
  `resources/read` access from clients that prefer the resource surface,
  but doubles the API contract. Deferred until at least two card types
  exist.
- Whether to support light/dark theme negotiation via an `_get_card`
  parameter or via the host's `prefers-color-scheme` exclusively.
  Currently the latter, server emits a single CSS block keyed on the
  media query.
