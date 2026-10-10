---
type: Test Case
title: "TC013: MCP Catalog View Permission"
description: "Verify that catalog visibility follows the MCP server view and manage permissions."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC013: MCP Catalog View Permission

## Description

Verify that users without MCP catalog permissions do not see the catalog in navigation and can still manage their personal connections.

## Preconditions

- API server and UI are running
- The current user satisfies neither `mcp_server.view` nor `mcp_server.manage`
- The current user has at least one personal MCP connection

## Steps

1. Sign in as the restricted user.
2. Navigate to Registries > MCP.
3. Inspect the available surfaces and network requests.
4. Remove the personal server from My MCP servers.

## Expected Result

- No MCP entry in the main navigation and no MCP catalog entry under Settings > Organization
- `/mcp-servers` redirects to `/settings/mcp-catalog`, which says the catalog is not available and links to My agent experience
- No catalog request is sent
- Add, edit, archive, and delete controls are not shown
- My MCP servers lists only the current user's servers and grants
- Removing the personal server also revokes its connection
