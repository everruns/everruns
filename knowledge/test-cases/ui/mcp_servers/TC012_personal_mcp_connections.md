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

1. Navigate to Settings > My agent experience.
2. Find My MCP servers.
3. Review the active preset row.
4. Review the deleted preset row.
5. Revoke the active preset's grant through the API (`DELETE /v1/user/connections/{provider}`) and reload.
6. Repeat the revoke request for the same provider through the API.

## Expected Result

- Only the current user's servers and grants are listed; there is no separate "MCP sign-ins for agent servers" section
- The active preset is one catalog row, Signed in with its date, offering Reconnect and Remove
- The deleted preset is labeled Preset unavailable, Server unavailable, and still offers Revoke
- After revoking, the active preset's row stays and shows Needs sign-in with Connect; another user's grant is unchanged
- Repeating revoke returns 204 and does not fail
