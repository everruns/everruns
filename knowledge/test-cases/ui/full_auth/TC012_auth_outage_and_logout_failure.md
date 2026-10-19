---
type: Test Case
title: "TC012: Full Auth - Auth Outage and Failed Sign-out"
description: "Verify that auth bootstrap failures block protected content and that failed sign-out clears application caches without falsely reporting the session as ended."
tags:
  - everruns
  - test-case
  - ui
  - full-auth
---
# TC012: Full Auth - Auth Outage and Failed Sign-out

## Description

Verify that auth bootstrap failures block protected content and that failed sign-out clears application caches without falsely reporting the session as ended.

## Preconditions

- Canonical `AUTH_MODE=full` stack and a signed-in disposable account with visible agents and sessions.
- Browser request interception can fail only selected auth endpoints; restore it after each variant.
- Preserve Network requests and observe whether requests are sent after the error state appears.

## Test Data

| Variant | Failure |
| --- | --- |
| Config outage | `/api/v1/auth/config`: 503 or network failure |
| User outage | `/api/v1/auth/me`: 503 or network failure |
| Signed out | `/api/v1/auth/me`: 401 |
| Logout outage | `/api/v1/auth/logout`: 503 |

## Steps

1. Reload `/agents` with the config-outage variant; inspect the page and requests.
2. Restore the endpoint and click **Retry**; verify recovery. Repeat with user outage.
3. Repeat using the signed-out variant, with no valid refresh credential.
4. Sign in again, load Agents and Sessions, then fail the logout endpoint and choose **Sign out**.
5. Inspect the error shown in the user menu and retry affordance; verify protected application caches and persisted org selection are cleared.
6. Restore logout and retry. Sign in as a second account and inspect its lists.

## Expected Result

- Config and non-401 user failures show **Authentication unavailable** with Retry; protected content stays blocked and the UI does not treat an outage as no-auth mode.
- Retry recovers when the endpoint is healthy. A real 401 follows the login flow instead of the outage flow.
- Failed sign-out displays failure and permits retry; it does not claim the server session has been revoked.
- Application data is cleared even on failure. Only the minimum current-user identity is retained for accurate retry UI.
- Successful retry clears cookies and cached identity. The next account cannot inherit the prior account's data.
