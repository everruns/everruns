# TC002: FCP POST plain text round-trip

## Description

Verify that posting plain text to the FCP endpoint produces a Markdown reply
from the agent, sets the `fcp_session` cookie, and that a follow-up POST with
that cookie resumes the same session.

## Preconditions

- API server running (`just start-dev`)
- A published app with an enabled `fcp` channel (`anonymous: true`)
- The app's agent uses a configured LLM provider (`llmsim` is fine)

## Test Data

| Field             | Value             |
| ----------------- | ----------------- |
| Channel type      | `fcp`             |
| `anonymous`       | `true`            |
| Body of POST #1   | `Hello, FCP!`     |
| Body of POST #2   | `Continue please` |

## Steps

1. From TC001, capture `APP_ID` of a published FCP app.

2. POST the first message:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" \
     --data 'Hello, FCP!'
   ```
   Capture the `Set-Cookie: fcp_session=...` response header as `COOKIE`.

3. POST a follow-up message with the cookie:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
     -H "Content-Type: text/plain" \
     -H "Cookie: $COOKIE" \
     --data 'Continue please'
   ```

4. Inspect the sessions list to confirm only one FCP session was created:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions" \
     | jq -r '.data[] | select(.tags[] | contains("fcp:app:'"$APP_ID"'")) | .id' | wc -l
   ```

## Expected Result

| Check                                                                  | Expected                                  |
| ---------------------------------------------------------------------- | ----------------------------------------- |
| POST #1 HTTP status                                                    | `200` (or `504` if the model is too slow) |
| POST #1 `Content-Type`                                                 | `text/markdown; charset=utf-8`            |
| POST #1 `Set-Cookie` includes `fcp_session=<uuid>; ...; HttpOnly`      | ✓                                         |
| POST #1 body is non-empty Markdown                                     | ✓                                         |
| POST #1 body contains no internal vocabulary (`tokio`, `openai`, etc.) | ✓                                         |
| POST #2 HTTP status                                                    | `200` or `504`                            |
| Sessions tagged `fcp:app:$APP_ID`                                      | exactly `1`                               |

## Validation Commands

```bash
# Capture cookie
COOKIE=$(curl -s -i -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
  -H 'Content-Type: text/plain' --data 'Hello' \
  | awk -F': ' '/^set-cookie:/{print $2}' | head -1 | tr -d '\r')

# Assert cookie shape
echo "$COOKIE" | grep -E '^fcp_session=[0-9a-f-]{36};' >/dev/null

# Assert second POST reuses the same session
curl -s -X POST "http://localhost:9300/api/v1/apps/$APP_ID/fcp" \
  -H 'Content-Type: text/plain' -H "Cookie: $COOKIE" --data 'Continue' >/dev/null

session_count=$(curl -s "http://localhost:9300/api/v1/sessions" \
  | jq -r --arg id "$APP_ID" '[.data[] | select(.tags[] | contains("fcp:app:\($id)"))] | length')
[ "$session_count" = "1" ]
```
