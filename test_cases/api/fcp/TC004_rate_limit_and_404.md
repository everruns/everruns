# TC004: FCP rate limiting and 404 sanitization

## Description

Verify two isolation invariants of the FCP endpoint:

1. The per-app rate limit returns `429` with a `Retry-After` header and an
   actionable Markdown body.
2. Requests for non-existent apps, unpublished apps, apps without an FCP
   channel, and apps with a disabled FCP channel all return an identical
   sanitized `404` body — the caller cannot distinguish operator state.

## Preconditions

- API server running (`just start-dev`)
- Three apps in different states:
  - `RL_APP_ID` — published, FCP enabled with `rate_limit_per_minute: 1`
  - `DRAFT_APP_ID` — created but never published
  - `WEBHOOK_APP_ID` — published with a webhook channel only (no FCP)

## Test Data

| Field                                | Value |
| ------------------------------------ | ----- |
| `rate_limit_per_minute` (`RL_APP_ID`)| `1`   |

## Steps

1. Burn the single allowed request on `RL_APP_ID`:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$RL_APP_ID/fcp" \
     -H 'Content-Type: text/plain' --data 'first'
   ```

2. Issue a second request that must be rate-limited:
   ```bash
   curl -i -X POST "http://localhost:9300/api/v1/apps/$RL_APP_ID/fcp" \
     -H 'Content-Type: text/plain' --data 'second'
   ```

3. Hit a completely unknown app id:
   ```bash
   curl -i "http://localhost:9300/api/v1/apps/app_does_not_exist/fcp"
   ```
   Save body to `/tmp/fcp_404_unknown.md`.

4. Hit the draft app's FCP endpoint:
   ```bash
   curl -i "http://localhost:9300/api/v1/apps/$DRAFT_APP_ID/fcp"
   ```
   Save body to `/tmp/fcp_404_draft.md`.

5. Hit the webhook-only app's FCP endpoint:
   ```bash
   curl -i "http://localhost:9300/api/v1/apps/$WEBHOOK_APP_ID/fcp"
   ```
   Save body to `/tmp/fcp_404_no_channel.md`.

## Expected Result

| Step | Check                                                                                   | Expected                                              |
| ---- | --------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| 2    | HTTP status                                                                             | `429`                                                 |
| 2    | `Retry-After` header                                                                    | `60`                                                  |
| 2    | `Content-Type`                                                                          | `text/markdown; charset=utf-8`                        |
| 2    | Body mentions `Rate limit` and the configured limit (`1 requests per minute`)           | ✓                                                     |
| 3-5  | HTTP status                                                                             | `404`                                                 |
| 3-5  | Body files are byte-identical (`diff` returns no diff)                                  | ✓                                                     |
| 3-5  | Body does NOT contain any of: `draft`, `unpublished`, `disabled`, `archived`, `channel` | ✓                                                     |

## Validation Commands

```bash
# Step 2
curl -s -o /tmp/fcp_429.md -D /tmp/fcp_429.h -w '%{http_code}\n' \
  -X POST "http://localhost:9300/api/v1/apps/$RL_APP_ID/fcp" \
  -H 'Content-Type: text/plain' --data 'second' | grep -q '^429$'
grep -i '^retry-after: 60' /tmp/fcp_429.h
grep -q 'Rate limit' /tmp/fcp_429.md
grep -q '1 requests per minute' /tmp/fcp_429.md

# Steps 3-5: 404 bodies must be identical
curl -s "http://localhost:9300/api/v1/apps/app_does_not_exist/fcp" >/tmp/a
curl -s "http://localhost:9300/api/v1/apps/$DRAFT_APP_ID/fcp"  >/tmp/b
curl -s "http://localhost:9300/api/v1/apps/$WEBHOOK_APP_ID/fcp" >/tmp/c
diff -q /tmp/a /tmp/b
diff -q /tmp/a /tmp/c

# Steps 3-5: no lifecycle leaks
! grep -E -i 'draft|unpublished|disabled|archived|channel' /tmp/a
```
