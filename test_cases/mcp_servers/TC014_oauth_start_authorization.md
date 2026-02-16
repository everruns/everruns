# TC014: Start OAuth Authorization (POST)

## Description

Verify that POST to the authorize endpoint returns a JSON response with the OAuth provider authorization URL including PKCE and resource indicator parameters.

## Preconditions

- API server is running (`just start-dev --no-watch`)
- An OAuth MCP server exists with explicit authorization_url and token_url configured
- `SECRETS_ENCRYPTION_KEY` is set (auto-set in dev mode)

## Test Data

| Field              | Value                              |
|--------------------|------------------------------------|
| Server Name        | test-oauth-server                  |
| Authorization URL  | https://example.com/authorize      |
| Token URL          | https://example.com/token          |
| Client ID          | test-client-id                     |
| Scopes             | ["read", "write"]                  |

## Steps

1. Create an OAuth MCP server (if not already existing):
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
2. POST to `/v1/mcp-servers/{server_uuid}/oauth/authorize`:
   ```bash
   curl -s -X POST http://localhost:9000/v1/mcp-servers/{server_uuid}/oauth/authorize \
     -H "Content-Type: application/json" \
     -d '{"return_url": "http://localhost:9300/settings/mcp-servers"}'
   ```

## Expected Result

- HTTP 200
- Response contains `authorization_url` field
- The URL starts with `https://example.com/authorize?`
- URL contains query parameters:
  - `client_id=test-client-id`
  - `redirect_uri=http://localhost:9000/v1/oauth/callback` (stable callback URL)
  - `response_type=code`
  - `state={uuid}` (CSRF state parameter)
  - `code_challenge={...}` (PKCE challenge)
  - `code_challenge_method=S256`
  - `scope=read write`
  - `resource=https://example.com/mcp` (RFC 8707)
