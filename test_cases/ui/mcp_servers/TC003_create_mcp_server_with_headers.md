# TC003: Create MCP Server - With Custom Headers

## Description

Verify that an MCP server can be created with custom HTTP headers for authentication.

## Preconditions

- API server is running
- User has access to MCP server management

## Test Data

| Field           | Value                              |
|-----------------|-------------------------------------|
| Name            | headers-mcp-server                 |
| URL             | https://headers.mcp.com/v1/mcp     |
| Headers         | X-Custom-Header: custom-value      |
|                 | X-Org-Id: org-12345                |

## Steps

1. Navigate to Settings > MCP Servers
2. Click "Add MCP Server" button
3. Enter name: `headers-mcp-server`
4. Enter URL: `https://headers.mcp.com/v1/mcp`
5. Add custom header: `X-Custom-Header: custom-value`
6. Add custom header: `X-Org-Id: org-12345`
7. Click "Create" button

## Expected Result

- MCP server is created successfully
- Server stores custom headers
- Headers are included in MCP server configuration
