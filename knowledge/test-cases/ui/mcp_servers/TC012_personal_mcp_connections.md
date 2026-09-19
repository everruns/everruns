---
type: Test Case
title: "TC012: Personal MCP Connections"
description: "Verify that users can inspect and revoke only their own MCP connections."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC012: Personal MCP Connections

## Description

Verify the current user's personal MCP connection list and idempotent revoke behavior.

## Preconditions

- API server and UI are running
- The current user has an OAuth grant for an active MCP preset
- Another user has a grant for the same preset
- The current user has a grant whose preset was deleted

## Steps

1. Navigate to Registries > MCP.
2. Open My connections.
3. Review the active preset row.
4. Review the deleted preset row.
5. Click Revoke on the active preset.
6. Repeat the revoke request for the same provider through the API.

## Expected Result

- Only the current user's grants are listed
- Each available preset shows its server, host, connected account, scopes, date, and state
- The deleted preset is labeled Preset unavailable and still offers Revoke
- Revoke removes the current user's grant without changing another user's grant
- Repeating revoke returns 204 and does not fail
