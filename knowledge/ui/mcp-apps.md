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

## Product surfaces

MCP Apps expose compact Agent, Harness, Capability and Knowledge collection cards,
plus the complete organization Home and Session workspaces. Session includes the
same Trace, Approvals, Work, Events, Files and Cost views as the browser UI.
The entry tools and resource identities live in the
[tool registry](../../crates/server/src/api/mcp_endpoint/tool_registry.rs).

Cards reuse Slate entity components; Home reuses `OrgHome`, and Session reuses
its provider, layout and tab components. The embedded route supplies the MCP
transport and identity contexts. It does not fork product layouts or require a
separate browser sign-in. Entity links open the real application through the
host; Session tab navigation stays inside the workspace. Fullscreen is requested
only when the host advertises it. Clipboard-write permission is declared for the
shared copy controls and delegated to the trusted frontend frame; hosts may deny it.

## Authentication and transport

The static MCP resource is a small host bridge. It frames the deployment's
configured frontend at `/mcp-app`, explicitly declaring that origin in its MCP
resource CSP. Deployments must serve that frontend for these views to render.
The child receives no bearer token or cookie-derived API identity. The embedded
provider proxies API requests through app-only MCP tools under the connected
caller's identity and resolved organization.

The server routes these requests in process to the existing REST handlers. A
private request extension carries the already authenticated caller; HTTP clients
cannot construct it, and MCP OAuth tokens remain invalid for the REST audience.
REST route scope classification, command policy, organization isolation and
feature gates still apply. Reads and writes have separate tools so read-only
credentials can load the workspace. The parent fixes the organization from the
entry result rather than accepting a child-selected organization. An app bridge
call may echo that already resolved organization, including with a PAT; selecting
a different organization retains the normal PAT override prohibition.

Session updates poll bounded event pages using a forward sequence cursor.
A since-id cursor is resolved within that session before paging, avoiding the
history endpoint's tail semantics. Busy pages drain before waiting for the next
poll. Closing the workspace stops listeners and timers; there is no direct SSE
connection carrying cookies.

## Security

- **XSS and framing.** The bridge uses no HTML insertion sinks. The child is the
  same React product UI and retains its existing content sanitization. The
  resource permits only the configured frontend frame; the embedded route's CSP
  restricts connections and resources to the deployment. Host and child messages
  are checked against the sending window, and child messages against its origin.
- **Authority.** Every data request returns through the host's MCP connection,
  normal organization resolution and the shared REST/command authorization path.
  The bridge accepts local product paths, bounded bodies and only explicit audit
  headers. It excludes auth, credential management, arbitrary commands, recursive
  MCP calls and external URLs. Session storage reads expose key values and secret
  metadata only, as the existing recording UI does; secret writes remain outside
  this bridge. The Session Credentials action opens Everruns through the host
  rather than collecting credential values inside the MCP frame. See TM-MCP-003, TM-MCP-004 and TM-MCP-009.
- **Resource bounds.** Tool deadlines and request/response limits reject oversized
  work explicitly. Events use bounded pages. Switching organization clears the
  query cache and rejects pending requests from the previous organization.
- **Approvals.** Existing question and approval actions retain their normal policy,
  validation and stale-action checks; embedding introduces no second resume path.

## Ownership and evidence

- [Resource shell and entrypoints](../../crates/server/src/api/mcp_endpoint/apps/workspace.rs)
- [Authenticated API bridge](../../crates/server/src/api/mcp_endpoint/apps/bridge.rs)
- [Embedded providers, routing and event transport](../../apps/ui/src/components/mcp-app/)
- [Integration coverage](../../crates/server/tests/domain/mcp_apps_test/workspace.rs)
- [Transport cursor and lifecycle tests](../../apps/ui/src/components/mcp-app/__tests__/transport.test.ts)

[Entity cards](mcp-cards.md) share this transport and authorization contract.
