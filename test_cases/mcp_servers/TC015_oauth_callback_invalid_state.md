# TC015: OAuth Callback with Invalid State

## Description

Verify that the OAuth callback endpoint rejects requests with invalid or expired state parameters.

## Preconditions

- API server is running (`just start-dev --no-watch`)

## Test Data

| Field | Value                                |
|-------|--------------------------------------|
| Code  | test-code                            |
| State | 00000000-0000-0000-0000-000000000000 |

## Steps

1. Send GET request to `/v1/oauth/callback` with a fabricated state:
   ```bash
   curl -s -w "\nHTTP: %{http_code}\n" \
     "http://localhost:9000/v1/oauth/callback?code=test-code&state=00000000-0000-0000-0000-000000000000"
   ```

## Expected Result

- HTTP 400
- Response body contains "Invalid or expired OAuth state"
