# TC007: Delete MCP Server

## Description

Verify that an MCP server can be deleted.

## Preconditions

- API server is running
- An MCP server exists that can be deleted

## Test Data

N/A

## Steps

1. Navigate to Settings > MCP Servers
2. Find the MCP server to delete
3. Click "Delete" button
4. Confirm the deletion

## Expected Result

- MCP server is deleted successfully
- Server no longer appears in the list
- Attempting to access the deleted server returns 404
