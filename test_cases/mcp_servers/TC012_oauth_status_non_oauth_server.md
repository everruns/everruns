# TC012: OAuth Status for Non-OAuth Server

## Description

Verify that the OAuth status endpoint returns `authorized: true` for servers that don't use OAuth.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- A non-OAuth MCP server exists (e.g., seeded `microsoft_learn`)

## Test Data

| Field     | Value                                      |
|-----------|--------------------------------------------|
| Server ID | 01933b5a-0000-7000-8000-000000000501 (MS Learn) |

## Steps

1. Send GET request to `/v1/mcp-servers/{server_id}/oauth/status`:
   ```bash
   curl -s http://localhost:9000/v1/mcp-servers/01933b5a-0000-7000-8000-000000000501/oauth/status
   ```

## Expected Result

- HTTP 200
- `auth_type` is `"none"`
- `authorized` is `true` (non-OAuth servers don't need authorization)
- `scopes` is empty
- `authorization_url` is null
