# TC013: OAuth Status for Unauthorized OAuth Server

## Description

Verify that the OAuth status endpoint returns `authorized: false` with an authorization URL when the user has not yet authorized an OAuth MCP server.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- An OAuth MCP server exists (e.g., seeded `github_copilot`)
- User has NOT authorized this server

## Test Data

| Field     | Value                                              |
|-----------|----------------------------------------------------|
| Server ID | 01933b5a-0000-7000-8000-000000000502 (GitHub Copilot) |

## Steps

1. Send GET request to `/v1/mcp-servers/{server_id}/oauth/status`:
   ```bash
   curl -s http://localhost:9000/v1/mcp-servers/01933b5a-0000-7000-8000-000000000502/oauth/status
   ```

## Expected Result

- HTTP 200
- `auth_type` is `"oauth"`
- `authorized` is `false`
- `authorization_url` is present and points to `/v1/mcp-servers/{id}/oauth/authorize`
- `scopes` is empty (no token yet)
