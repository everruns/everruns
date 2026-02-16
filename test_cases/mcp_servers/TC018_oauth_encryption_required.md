# TC018: OAuth Requires Encryption Service

## Description

Verify that OAuth endpoints return an appropriate error when encryption is not configured (no `SECRETS_ENCRYPTION_KEY` set).

## Preconditions

- API server is running WITHOUT `SECRETS_ENCRYPTION_KEY` set
- An OAuth MCP server exists

## Steps

1. Start the server without encryption key:
   ```bash
   unset SECRETS_ENCRYPTION_KEY
   DEV_MODE=true cargo run -p everruns-server
   ```
2. Request OAuth status for an OAuth server:
   ```bash
   curl -s http://localhost:9000/v1/mcp-servers/{server_uuid}/oauth/status
   ```

## Expected Result

- HTTP 500
- Response contains `{"error": "OAuth not configured (encryption not enabled)"}`
- OAuth features are disabled when encryption is not available

## Notes

- In `just start-dev` and `just start-all`, `SECRETS_ENCRYPTION_KEY` is auto-set with a default dev key
- This test verifies graceful degradation when encryption is missing
