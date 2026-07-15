# TC009: Create MCP Server - Validation: Empty URL

## Description

Verify that the create action remains unavailable while the required URL is empty.

## Preconditions

- API server is running

## Test Data

| Field | Value           |
|-------|-----------------|
| Name  | test-server    |
| URL   | (empty)        |

## Steps

1. Navigate to Building blocks > MCP Servers
2. Click "Add MCP Server" button
3. Enter name: `test-server`
4. Leave URL field empty
5. Observe the "Create Server" button

## Expected Result

- "Create Server" is disabled
- No create request is sent
- Entering a valid absolute URL enables the button when all other required fields are valid
