# TC017: OAuth Callback Uses Single Stable Path

## Description

Verify that all OAuth callbacks use the single stable path `/v1/oauth/callback` instead of per-server paths. The MCP server ID is recovered from the `state` parameter stored in the database.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- An OAuth MCP server exists with explicit OAuth config

## Steps

1. Create an OAuth MCP server and POST to authorize (see TC014)
2. Inspect the `redirect_uri` in the authorization URL query parameters
3. Verify the callback endpoint responds at `/v1/oauth/callback`

## Expected Result

- The `redirect_uri` in the authorization URL is `http://localhost:9000/v1/oauth/callback`
  - NOT `http://localhost:9000/v1/mcp-servers/{id}/oauth/callback`
- Only one redirect URI needs to be registered with any OAuth provider
- The `state` parameter maps back to the correct MCP server ID
