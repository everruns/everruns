# TC002: Create MCP Server - With API Key

## Description

Verify that an MCP server can be created with an API key for authentication.

## Preconditions

- API server is running
- User has access to MCP server management

## Test Data

| Field   | Value                              |
|---------|-------------------------------------|
| Name    | secure-mcp-server                  |
| URL     | https://secure.mcp.com/v1/mcp      |
| API Key | sk-test-12345                      |

## Steps

1. Navigate to Settings > MCP Servers
2. Click "Add MCP Server" button
3. Enter name: `secure-mcp-server`
4. Enter URL: `https://secure.mcp.com/v1/mcp`
5. Enter API key: `sk-test-12345`
6. Click "Create" button

## Expected Result

- MCP server is created successfully
- Server appears in the MCP servers list
- api_key_set is true
- API key is stored encrypted (not visible in response)
