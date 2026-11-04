---
title: OAuth MCP servers with path-specific metadata now connect
description: The MCP connection check and sign-in now find OAuth metadata at every location the MCP authorization spec allows, so these servers no longer read unreachable.
date: 2026-10-17
kind: fix
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [ui, agent]
links:
  docs: [/features/mcp/]
---

Some remote MCP servers that sign in with OAuth showed **The server's sign-in
service did not answer correctly** in the MCP catalog, and their sign-in
failed. Everruns looked for OAuth metadata in only one place per server. A
server that published it only at its endpoint path (for example
`/.well-known/oauth-protected-resource/mcp`), only as OpenID Connect
configuration, or not at all was reported as unreachable.

The connection check and sign-in now try each location the MCP authorization
spec lists, in order. A server with no protected-resource metadata is treated
as its own sign-in service. Select **Check again** on an affected catalog entry
to record the new result. See
[Everruns as an MCP client](/features/mcp/#everruns-as-an-mcp-client).
