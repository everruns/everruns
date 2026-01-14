# TC006: Disable MCP Server

## Description

Verify that an MCP server can be disabled.

## Preconditions

- API server is running
- An active MCP server exists

## Test Data

| Field  | Value    |
|--------|----------|
| Status | disabled |

## Steps

1. Navigate to Settings > MCP Servers
2. Find an active MCP server
3. Click "Disable" or toggle status
4. Confirm the action

## Expected Result

- MCP server status changes to "disabled"
- Disabled server is still visible in the list
- Disabled server is not used for agent tool calls
