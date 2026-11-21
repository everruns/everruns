---
type: Specification
title: "MCP Apps host: showing connected servers' views"
description: "Everruns as an MCP Apps (SEP-1865) host: views a connected server ships render under their tool call in the console, as a picture plus link in Slack, sandboxed and unable to act on their own authority."
tags:
  - everruns
  - mcp
  - integrations
  - ui
---
# MCP Apps host: showing connected servers' views

Status: implemented (2026-11-20). Spec: [MCP Apps, SEP-1865](https://github.com/modelcontextprotocol/ext-apps), protocol `2026-01-26`.

## Why

MCP servers such as Sentry return a chart or a table as an interactive view
next to a tool's text result. Before this, Everruns dropped the view and kept
only the text. The feature is generic: any connected server that declares
`_meta.ui.resourceUri` on a tool gets its view shown.

## What happens

1. The runtime client advertises the `io.modelcontextprotocol/ui` extension
   (`with_ui_apps`, enabled on the worker host). Tools whose `visibility` leaves
   out `model` are app-only: they never reach the model's tool list.
2. A call to a tool with a view records `mcp_app` (server, tool, `ui://` URI,
   structured content, `_meta`) in the tool result's `details`, which the model
   never sees. The model's text result is unchanged.
3. The console reads `tool.completed.details.mcp_app`, loads the view's HTML
   through the session's MCP connection when the row renders (never stored in
   events), and runs it in an iframe as the host side of the bridge. See
   `apps/ui/src/components/mcp/mcp-app-frame.tsx` and
   `crates/server/src/domains/session_commands/mcp_apps.rs`.
4. Slack's channel delivery hands the view to the channel's app-view surface:
   a PNG taken by Browserless (the session's `browserless` connection) plus an
   "Open interactive view" button to the session. Without Browserless only the
   button is posted.

## Decisions

- **Sandbox.** `srcdoc` with `sandbox="allow-scripts"` and no
  `allow-same-origin`: an opaque origin with no access to cookies, storage or
  the console DOM. A CSP meta tag built only from the https/wss origins the view
  declared goes first in its head; anything else is dropped twice (server and
  browser).
- **Known limit: inherited CSP.** A `srcdoc` frame also inherits the console's
  own CSP, so on a deployment whose `connect-src` is `'self'` a view that fetches
  from its declared origins, or loads a script from a CDN, is blocked. Inline
  single-file views work. A separate sandbox origin would lift this; not built.
- **Views ask, the console decides.** `tools/call` from a view: tools marked
  `readOnlyHint` or app-only run directly; any other tool returns
  `confirmation_required` and the console asks the person; a tool hidden from
  apps is refused. `ui/message` only fills the composer as a draft.
  `ui/open-link` accepts http(s) only and opens with `noopener`.
- **Owner only.** Only the session's owner (or an internal caller) lets a view
  call tools or read non-view resources. Others see the view fed with the data
  the call already returned.
- **Not offered:** `ui/update-model-context`, sampling and `ui/download-file`,
  so a view cannot steer the model. Messages are rate limited per frame.
- **Source always named.** The server's name sits above the frame so a view
  cannot pass itself off as console UI.
- **Per-server setting** `interactive_views`: `on` (console and Slack),
  `console_only`, `off`. Stored in the server's settings JSON, default `on`
  and omitted when default, so no migration.
- **Failure is visible.** A skeleton holds the place until the view says it is
  initialized; a load error or a 10 second stall shows the text result with
  Retry.

## Related

- [MCP Server Specification](mcp-servers.md)
- [MCP Resources](mcp-resources.md)
- Threat TM-WEB-027 in [the threat model](../security/threat-model.md)
