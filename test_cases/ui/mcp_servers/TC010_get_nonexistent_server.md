# TC010: Get Non-existent MCP Server

## Description

Verify that attempting to get a non-existent MCP server returns 404.

## Preconditions

- API server is running

## Test Data

| Field     | Value                                  |
|-----------|----------------------------------------|
| Server ID | mcp_00000000-0000-0000-0000-000000000000 |

## Steps

1. Make GET request to `/api/v1/mcp-servers/mcp_00000000-0000-0000-0000-000000000000`

## Expected Result

- HTTP status 404 (Not Found) is returned
- Response indicates server not found
- A bare UUID is outside the resource ID contract and should return 400 instead
