---
type: Specification
title: "MCP Apps: Everruns in ChatGPT, Codex and Claude"
description: "Interactive MCP Apps views on the /mcp server: session view, questions, approvals, and the home panel, for ChatGPT, Codex and any MCP Apps host."
tags:
  - everruns
  - ui
  - mcp
---
# MCP Apps: Everruns in ChatGPT, Codex and Claude

## Why

ChatGPT and Codex turned plugins into apps at DevDay 2026: a plugin is an MCP
server plus UI built on MCP Apps, with OpenAI extensions for a sidebar entry and
a thread tab. Claude renders the same MCP Apps standard. Everruns already serves
MCP at `/mcp` (see [MCP](../integrations/mcp.md)), so adding views there puts
agent runs, questions and approvals inside those hosts without a separate
integration per host. Publishing to OpenAI's plugin directory is a SaaS listing
decision and out of scope here; this makes any deployment ready for it.

## What a host needs from us

Pinned to the specs we implemented against (2026-09-30):

- **MCP Apps**, SEP-1865 draft (`modelcontextprotocol/ext-apps`,
  `specification/draft/apps.mdx`). A tool names a template in
  `_meta.ui.resourceUri`; the host reads it with `resources/read`, MIME
  `text/html;profile=mcp-app`, renders it in a sandboxed iframe, and talks to it
  over postMessage JSON-RPC (`ui/initialize`, `ui/notifications/tool-result`,
  `tools/call`, `ui/open-link`, `ui/request-display-mode`). `_meta.ui.visibility:
  ["app"]` hides a tool from the model while keeping it callable from the view.
  Template metadata (`_meta.ui.csp`, `prefersBorder`) rides on the resource.
- **OpenAI MCP extensions** (`openai/mcp-extensions`, `docs/spec.md`). On top of
  MCP Apps: `_meta["openai/ui"].entrypoints` (`global` sidebar entry, `thread`
  tab; both open the tool with `{}`), tool `icons` (monochrome 20x20 SVG using
  `currentColor`), and `_meta["openai/ui"]` display modes on the resource
  (`availableDisplayModes`, `preferredDisplayMode`; ChatGPT supports `inline` and
  `fullscreen`, not `pip`). Settings, file handlers and at-mentions are not used.
- **Discovery.** MCP 2026-07-28 clients call `server/discover` instead of
  `initialize`; it advertises `extensions["io.modelcontextprotocol/ui"]` next to
  Tasks. Older clients get the same extension map from `initialize` under
  2026-07-28 only.
- **Auth.** ChatGPT connects with OAuth 2.1 + PKCE S256, the `resource`
  parameter bound into the token audience, and either Client ID Metadata
  Documents or Dynamic Client Registration. `/mcp` already mints
  audience-bound `mcp_access` tokens and supports DCR (see
  [MCP OAuth](../integrations/mcp.md)); the protected-resource metadata now also
  lists `scopes_supported`. CIMD is not implemented; ChatGPT falls back to DCR.

## Views

One template document serves two URIs, so both stay static and cacheable and
all data arrives as `structuredContent`:

| Template | Opened by | Shows |
|---|---|---|
| `ui://everruns/app/session` | `agent_run`, `session_send_message`, `session_get_status` results | Status, recent messages, the pending `ask_user` question as a form, the pending `request_approval` as Approve/Decline, a reply box when idle, "Open in Everruns" |
| `ui://everruns/app/home` | `everruns_home` (sidebar entry and thread tab) | Agents with a Run box, recent sessions, and everything waiting on the user |

The view polls `session_view` while a turn runs and stops when the session is
idle or waiting on the user. It branches on what the host granted in
`ui/initialize` (`serverTools`, `openLinks`, available display modes), never on
which host it is: a host that cannot proxy tool calls gets a read-only view.

### Tools

`everruns_home` is model-visible so the model can open the panel. The action
tools are `visibility: ["app"]`:

- `session_view`: the view state.
- `session_answer_question`: answers go through the shared
  `resolve_question_answers` operation, the same one the REST endpoint and form
  elicitation use, so labels are validated against what was actually asked.
  Secret questions are never rendered as a form; the view links to Everruns.
- `session_decide_approval`: the decision becomes the user's next message
  (`Approved: …` / `Declined: …`), exactly like a Slack click
  ([Soft Approval](../execution/soft-approval.md)), so approval audit attributes
  it to the MCP user. The view echoes the action it showed and a mismatch is
  refused as stale.

An MCP Apps host also counts as able to answer questions, so `agent_run` sets
the `ask_user` pause hint for it the same way it does for a client that
declared form elicitation.

## Security

- **XSS.** The template writes every server value with `textContent`; a unit test
  keeps `innerHTML` and friends out of it. The document carries a `<meta>` CSP
  with `connect-src 'none'`, and the resource declares no CSP domains, so a
  host applies the spec's restrictive default. See TM-MCP-003.
- **Widget-to-server calls (CSRF, token scope).** The view has no credential and
  no network. Every action is a host-proxied `tools/call` on `/mcp` with the
  user's own audience-bound MCP token, through the normal org resolution and
  command policy (`SESSION_VIEW` to read, `SESSION_MANAGE` to answer or
  decide). There is no cookie or ambient credential a cross-site request could
  ride. See TM-MCP-004 and TM-MCP-009.
- **Stale or replayed clicks.** A question answer names the `tool_call_id` it
  answers and is single-claim; an approval must match the currently pending
  action on an idle session. See TM-MCP-009.

## Code

`crates/server/src/api/mcp_endpoint/apps.rs` (metadata, view state, actions),
`apps/app.html` (the template), `resources.rs` (`resources/list|read`),
`tool_registry.rs` (tool descriptors). The embedded-resource
[entity cards](mcp-cards.md) predate MCP Apps and stay as they are.
