# TC008: Create MCP Server - Validation: Empty Name

## Description

Verify that the create action remains unavailable while the required name is empty.

## Preconditions

- API server is running

## Test Data

| Field | Value                           |
|-------|----------------------------------|
| Name  | (empty)                         |
| URL   | https://mcp.example.com/v1/mcp  |

## Steps

1. Navigate to Registries > MCP servers
2. Click "Add MCP Server" button
3. Leave name field empty
4. Enter URL: `https://mcp.example.com/v1/mcp`
5. Observe the "Create Server" button

## Expected Result

- "Create Server" is disabled
- No create request is sent
- Entering a non-empty name enables the button when all other required fields are valid
