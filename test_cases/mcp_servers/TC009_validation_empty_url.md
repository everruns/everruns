# TC009: Create MCP Server - Validation: Empty URL

## Description

Verify that creating an MCP server with an empty URL fails validation.

## Preconditions

- API server is running

## Test Data

| Field | Value           |
|-------|-----------------|
| Name  | test-server    |
| URL   | (empty)        |

## Steps

1. Navigate to Settings > MCP Servers
2. Click "Add MCP Server" button
3. Enter name: `test-server`
4. Leave URL field empty
5. Click "Create" button

## Expected Result

- Creation fails with validation error
- Error message indicates "URL cannot be empty"
- HTTP status 400 (Bad Request) is returned
