# TC006: Archive MCP Server

## Description

Verify that an MCP server can be archived and remains available in the archived view.

## Preconditions

- API server is running
- An active MCP server exists

## Test Data

| Field  | Value    |
|--------|----------|
| Status | archived |

## Steps

1. Navigate to Building blocks > MCP Servers
2. Find an active MCP server
3. Open the server actions and click "Archive"
4. Confirm the action
5. Open the Archived tab

## Expected Result

- MCP server is removed from the active list
- MCP server appears in the Archived tab with archived status
- Archived server is not used for agent tool calls
