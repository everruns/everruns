# TC016: Revoke OAuth Token When None Exists

## Description

Verify that revoking an OAuth token for a server where the user has no token returns 404.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- An OAuth MCP server exists
- User has NOT authorized this server

## Test Data

| Field     | Value                                              |
|-----------|----------------------------------------------------|
| Server ID | 01933b5a-0000-7000-8000-000000000502 (GitHub Copilot) |

## Steps

1. Send DELETE request to `/v1/mcp-servers/{server_uuid}/oauth/token`:
   ```bash
   curl -s -w "\nHTTP: %{http_code}\n" -X DELETE \
     http://localhost:9000/v1/mcp-servers/01933b5a-0000-7000-8000-000000000502/oauth/token
   ```

## Expected Result

- HTTP 404
- Response contains `{"error": "No token found"}`
