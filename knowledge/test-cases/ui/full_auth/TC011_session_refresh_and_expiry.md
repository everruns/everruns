---
type: Test Case
title: "TC011: Full Auth - Session Renewal and Expiry"
description: "Verify that an idle browser session renews through its refresh cookie and that expired refresh credentials return the user to login without losing the destination."
tags:
  - everruns
  - test-case
  - ui
  - full-auth
---
# TC011: Full Auth - Session Renewal and Expiry

## Description

Verify that an idle browser session renews through its refresh cookie and that expired refresh credentials return the user to login without losing the destination.

## Preconditions

- Canonical authenticated stack with `AUTH_MODE=full`, a verified account, and an existing agent.
- Browser tooling can expire only the disposable access cookie while retaining refresh and organization cookies.
- Use the Network panel to distinguish overlapping requests from later retries.

## Test Data

| Variant | Credentials |
| --- | --- |
| Idle session | Access cookie absent; refresh cookie valid |
| Expired session | Access cookie absent; refresh cookie expired or revoked |
| Destination | `/agents/<owned-agent-id>` |

## Steps

1. Sign in, open the destination, and expire the access cookie only.
2. Reload the protected URL. Observe the data requests and refresh response.
3. Repeat with several overlapping non-auth API reads in the same tab, all receiving 401 before refresh completes.
4. Confirm the retried reads load the same account and organization; reload again.
5. Repeat with the expired-session fixture. Inspect the refresh failure and resulting login URL.
6. Sign in again and verify continuation to the original agent.

## Expected Result

- A refresh-only browser passes the route proxy and renews instead of prematurely redirecting to login.
- Overlapping reads share one in-flight refresh; successful reads retry once with the new access cookie.
- Renewal preserves account, organization, and destination.
- Rejected refresh cannot restore access. Protected content remains blocked and login preserves a sanitized `return_to` destination.
- There is no unbounded refresh/retry loop. Requests to auth endpoints do not recursively refresh themselves.

## Review Note

Source review identified a path to verify: the route proxy admits refresh-only cookies, but the client skips automatic refresh for `/v1/auth/*`, including the bootstrap `/v1/auth/me` request. Cold reload must be exercised separately from concurrent non-auth reads. A login bounce with a valid refresh cookie fails this case; source review alone is not execution evidence.
