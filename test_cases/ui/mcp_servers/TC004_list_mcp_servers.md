# TC004: List MCP Servers

## Description

Verify that MCP servers can be listed and viewed.

## Preconditions

- API server is running
- At least one MCP server exists in the system

## Test Data

N/A

## Steps

1. Navigate to Settings > MCP Servers
2. View the MCP servers list

## Expected Result

- All MCP servers are displayed in the list
- Each server shows: name, URL, status, and creation date
- Servers are sorted by creation date (newest first)
- API key status is visible (set/not set) but not the actual key
