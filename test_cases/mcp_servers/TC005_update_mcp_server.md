# TC005: Update MCP Server

## Description

Verify that an existing MCP server can be updated.

## Preconditions

- API server is running
- An MCP server exists with name "test-mcp-server"

## Test Data

| Field       | Original Value                | Updated Value                  |
|-------------|-------------------------------|--------------------------------|
| Name        | test-mcp-server              | updated-mcp-server             |
| Description | Original description          | Updated description            |
| URL         | https://old.mcp.com/v1/mcp   | https://new.mcp.com/v1/mcp    |

## Steps

1. Navigate to Settings > MCP Servers
2. Find the server "test-mcp-server"
3. Click "Edit" button
4. Update name to: `updated-mcp-server`
5. Update description to: `Updated description`
6. Update URL to: `https://new.mcp.com/v1/mcp`
7. Click "Save" button

## Expected Result

- MCP server is updated successfully
- List shows updated values
- updated_at timestamp is updated
