# TC011: Create MCP Server with OAuth Authentication

## Description

Verify that an MCP server with OAuth authentication can be created with explicit OAuth configuration.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- User has access to MCP server management

## Test Data

| Field              | Value                              |
|--------------------|------------------------------------|
| Name               | test-oauth-server                  |
| URL                | https://example.com/mcp            |
| Auth Type          | oauth                              |
| Authorization URL  | https://example.com/authorize      |
| Token URL          | https://example.com/token          |
| Client ID          | test-client-id                     |
| Scopes             | ["read", "write"]                  |

## Steps

1. Send POST request to `/v1/mcp-servers`:
   ```bash
   curl -s -X POST http://localhost:9000/v1/mcp-servers \
     -H "Content-Type: application/json" \
     -d '{
       "name": "test-oauth-server",
       "url": "https://example.com/mcp",
       "auth_type": "oauth",
       "oauth_config": {
         "authorization_url": "https://example.com/authorize",
         "token_url": "https://example.com/token",
         "client_id": "test-client-id",
         "scopes": ["read", "write"]
       }
     }'
   ```
2. Verify the response

## Expected Result

- HTTP 201 Created
- Response contains `auth_type: "oauth"`
- Response contains `oauth_config` with `authorization_url`, `token_url`, `client_id`, `scopes`
- `client_secret_set` is `false` (no secret provided)
- Server appears in list with OAuth configuration
