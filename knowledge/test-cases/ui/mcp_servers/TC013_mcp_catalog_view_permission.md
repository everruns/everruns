---
type: Test Case
title: "TC013: MCP Catalog View Permission"
description: "Verify that catalog visibility follows the MCP server view permission."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC013: MCP Catalog View Permission

## Description

Verify that users without MCP catalog view permission can still manage their personal connections.

## Preconditions

- API server and UI are running
- The current user does not satisfy `mcp_server.view`
- The current user has at least one personal MCP connection

## Steps

1. Sign in as the restricted user.
2. Navigate to Registries > MCP.
3. Inspect the available surfaces and network requests.
4. Revoke the personal connection.

## Expected Result

- The Catalog surface is not shown
- No catalog request is sent
- Add, edit, archive, and delete controls are not shown
- My connections remains visible and lists only the current user's grants
- The personal connection can be revoked
