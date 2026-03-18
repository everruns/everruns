# TC001: Create MCP Server - Basic Creation

## Description

Verify that an MCP server can be created with required fields (name, URL).

## Preconditions

- API server is running
- User has access to MCP server management (Settings page)

## Test Data

| Field       | Value                              |
|-------------|-------------------------------------|
| Name        | atlassian-mcp-server               |
| Description | Atlassian MCP Server for Jira      |
| URL         | https://mcp.atlassian.com/v1/mcp   |

## Steps

1. Navigate to Settings > MCP Servers
2. Click "Add MCP Server" button
3. Enter name: `atlassian-mcp-server`
4. Enter description: `Atlassian MCP Server for Jira`
5. Enter URL: `https://mcp.atlassian.com/v1/mcp`
6. Click "Create" button

## Expected Result

- MCP server is created successfully
- Server appears in the MCP servers list
- Server status is "active"
- Server shows transport type "http"
- api_key_set is false (no API key provided)
