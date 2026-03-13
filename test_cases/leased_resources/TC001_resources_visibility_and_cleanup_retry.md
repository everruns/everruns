# Description

Verify leased resources appear in the session Resources surface, survive the full-mode persistence path, and transition through durable cleanup retry states without exposing provider secrets in metadata.

# Preconditions

- Branch includes the leased-resources feature.
- `just start-dev --no-watch` works locally.
- `just start-all --no-watch --no-ui` works locally with PostgreSQL.
- Browserless and Daytona capabilities are registered in the build.

# Test Data

| Name | Value |
| --- | --- |
| Harness | Platform Chat harness |
| Session capability | `browserless` |
| Resource provider | `browserless` |
| Resource type | `browser_session` |
| External ID | `browser-lease-smoke` |

# Steps

1. Start dev mode and create a session with the `browserless` capability.
2. Open the session page and confirm the Resources tab is visible.
3. Open the Resources tab and confirm the empty state renders when no leases exist.
4. Stop dev mode.
5. Start full mode with PostgreSQL and create another session with the `browserless` capability.
6. Insert a leased resource row for that session with expired `lease_expires_at`, `metadata.ws_endpoint` set to a tokenless reconnect endpoint, and provider/type set to `browserless` / `browser_session`.
7. Call `GET /v1/sessions/{session_id}/resources` and confirm the inserted resource is returned.
8. Confirm the API response metadata does not include bearer tokens or provider API keys.
9. Wait for the durable `leased-resource-cleanup` schedule to run.
10. Call the resources API again and confirm the resource transitions to `cleanup_failed` with a retry deadline if cleanup cannot complete.
11. Inspect durable schedule state or worker logs and confirm the cleanup execution is observable.

# Expected Result

- The Resources tab is available for sessions whose capabilities expose `leased_resources`.
- Empty-state UI renders correctly when no leases are present.
- Full mode persists leased resources and returns them through the session resources API.
- Cleanup attempts are driven by the durable scheduler and recorded in schedule history/logs.
- API-visible leased-resource metadata contains only non-secret values.
