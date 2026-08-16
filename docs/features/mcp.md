---
title: Model Context Protocol (MCP)
description: Everruns is both an MCP server, exposing its agents and tools to external clients with OAuth 2.1, and an MCP client that registers remote servers as virtual capabilities.
sidebar:
  label: MCP
---

Everruns speaks the [Model Context Protocol](https://spec.modelcontextprotocol.io) on **both sides**: it exposes its own agents and tools as an MCP server, and it consumes remote MCP servers as agent capabilities.

## Everruns as an MCP server

Every Everruns deployment exposes an authenticated MCP endpoint at `/mcp` so external clients, Claude Desktop, Cursor, VS Code, or another agent, can discover and call your agents and tools over JSON-RPC. It is a standard product surface, with no deployment variable or organization feature toggle.

- **Transport**: JSON-RPC 2.0 over Streamable HTTP (`POST /mcp`). Protocol versions `2025-06-18` and `2025-03-26` are supported.
- **Methods**: `initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`.
- **Auth**: MCP requests authenticate and resolve an organization before dispatch. OAuth 2.1 uses mandatory PKCE, and MCP access tokens are bound to the exact `/mcp` resource so they cannot be reused against the REST API. Unauthenticated requests fail with `401` and protected-resource discovery metadata is served at `/.well-known/oauth-protected-resource/mcp`.
- **Entity cards**: under protocol `2025-06-18`, tools like `agent_get_card` return a sandboxed `text/html` MCP App resource at the `ui://` scheme alongside a text summary, so MCP-Apps-aware hosts render rich cards while others fall back to text.

Routing is intentionally split: REST under `/api/*`, MCP OAuth under `/oauth/*`, and MCP JSON-RPC at `/mcp`.

## Everruns as an MCP client

Register a remote MCP server and its tools appear as a **virtual capability**: auto-discovered, namespaced, and executed alongside built-in capabilities. No code changes are needed to give an agent new tools.

- **Org-managed servers**: organization-scoped `McpServer` records connect over remote HTTP (Streamable HTTP). `stdio` is rejected by the hosted control plane and is only available to single-tenant runtime/CLI hosts.
- **Scoped `mcpServers`**: harnesses, agents, and sessions can embed remote MCP config directly (the remote-server subset of `.mcp.json`) for session-local or agent-local wiring without creating an org-global record.
- **Tool naming**: discovered tools are namespaced per server so they never collide with built-in capabilities.
- **Protocol compatibility**: the client negotiates the MCP protocol era per server. By default (`auto`) it issues a session-less `2026-07-28` request and transparently falls back to the stateful `initialize` handshake (`2025-06-18` / `2025-03-26`) for servers that require it, caching the verdict per server. Set the protocol mode to `legacy`, `stable`, or `rc` to pin a specific era and skip negotiation. No setting is needed for the common case.

## Use Everruns from your AI tools

To connect Claude Code, Codex, or Cursor to a deployment via the `everruns-dev` plugin, see [Use in AI tools](/getting-started/use-in-ai-tools/).

## Related

- [Capabilities](/features/capabilities/), how virtual capabilities fit the capability system
- [Apps](/features/apps/), publishing agents to channels
