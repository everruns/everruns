# TC003: FCP token authentication

## Description

Verify that an FCP channel with a configured shared token rejects requests
without the token and accepts requests bearing the correct token via either
`Authorization: Bearer` or `X-Everruns-FCP-Token`. The token must never be
echoed back in any response body and must use constant-time comparison.

## Preconditions

- API server running (`just start-dev`)
- A published app with an enabled `fcp` channel configured with a token

## Test Data

| Field         | Value                                  |
| ------------- | -------------------------------------- |
| Channel type  | `fcp`                                  |
| `anonymous`   | `true`                                 |
| `token`       | `fcp-secret-12345`                     |

## Steps

1. Create the app with a token configured:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/apps" \
     -H "Content-Type: application/json" \
     -d '{
       "name":"Token Protected FCP",
       "harness_id":"<base harness id>",
       "agent_id":"<agent id>",
       "channel_type":"fcp",
       "channel_config":{"anonymous":true,"token":"fcp-secret-12345"}
     }'
   ```
   Save `id` as `APP_ID`, publish it.

2. POST without a token:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" --data 'hi'
   ```

3. POST with a **wrong** token:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" \
     -H "Authorization: Bearer wrong-token" \
     --data 'hi'
   ```

4. POST with the **correct** token via `Authorization`:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" \
     -H "Authorization: Bearer fcp-secret-12345" \
     --data 'hi'
   ```

5. POST with the **correct** token via the dedicated header:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" \
     -H "X-Everruns-FCP-Token: fcp-secret-12345" \
     --data 'hi'
   ```

6. Read the app and confirm the token field is redacted:
   ```bash
   curl -s "http://localhost:9300/api/v1/apps/$APP_ID" | jq '.channels[] | select(.channel_type=="fcp") | .channel_config'
   ```

## Expected Result

| Step | Check                                                              | Expected                                                                  |
| ---- | ------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| 2    | HTTP status                                                        | `401`                                                                     |
| 2    | `Content-Type`                                                     | `text/markdown; charset=utf-8`                                            |
| 2    | Body contains actionable guidance                                  | mentions `Authorization: Bearer` and `X-Everruns-FCP-Token`               |
| 2    | Body does NOT contain the token                                    | `fcp-secret-12345` MUST NOT appear in any body                            |
| 3    | HTTP status                                                        | `401` (same body as step 2 — no oracle on validity)                       |
| 4    | HTTP status                                                        | `200` or `504` (request accepted; model may or may not reply in time)     |
| 5    | HTTP status                                                        | `200` or `504`                                                            |
| 6    | App read response                                                  | `token` absent; `token_configured: true` present; no plaintext token leak |

## Validation Commands

```bash
# Step 2: 401 and no token in body
status=$(curl -s -o /tmp/fcp_401.md -w '%{http_code}' \
  -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
  -H 'Content-Type: text/plain' --data 'hi')
[ "$status" = "401" ]
grep -q 'Authorization: Bearer' /tmp/fcp_401.md
grep -q 'X-Everruns-FCP-Token' /tmp/fcp_401.md
! grep -q 'fcp-secret-12345' /tmp/fcp_401.md

# Step 4: accepted
status=$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
  -H 'Content-Type: text/plain' \
  -H 'Authorization: Bearer fcp-secret-12345' --data 'hi')
echo "$status" | grep -E '^(200|504)$'

# Step 6: redaction
curl -s "http://localhost:9300/api/v1/apps/$APP_ID" \
  | jq -e '.channels[] | select(.channel_type=="fcp") | .channel_config |
           (has("token") | not) and (.token_configured == true)'
```
