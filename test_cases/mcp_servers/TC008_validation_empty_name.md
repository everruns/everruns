# TC008: Create MCP Server - Validation: Empty Name

## Description

Verify that creating an MCP server with an empty name fails validation.

## Preconditions

- API server is running

## Test Data

| Field | Value                           |
|-------|----------------------------------|
| Name  | (empty)                         |
| URL   | https://mcp.example.com/v1/mcp  |

## Steps

1. Navigate to Settings > MCP Servers
2. Click "Add MCP Server" button
3. Leave name field empty
4. Enter URL: `https://mcp.example.com/v1/mcp`
5. Click "Create" button

## Expected Result

- Creation fails with validation error
- Error message indicates "Name cannot be empty"
- HTTP status 400 (Bad Request) is returned
