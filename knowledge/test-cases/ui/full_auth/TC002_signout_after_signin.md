---
type: Test Case
title: "TC002: Full Auth - Sign-out and Account Isolation"
description: "Verify successful sign-out revokes refresh, clears account data, and requires authentication before protected pages can load again."
tags:
  - everruns
  - test-case
  - ui
  - full-auth
---
# TC002: Full Auth - Sign-out and Account Isolation

## Description

Verify successful sign-out revokes refresh, clears account data, and requires authentication before protected pages can load again.

## Preconditions

- Canonical stack with `AUTH_MODE=full` and password auth enabled.
- Two disposable, verified accounts A and B with distinct organizations and agents.
- Browser Network panel preserves requests; record credentials only in private test tooling.

## Test Data

| Field | Value |
| --- | --- |
| Account A | Fresh verified account with agent `account-a-proof` |
| Account B | Different verified account without access to A's organization |
| Protected destination | `/agents` |

## Steps

1. At `/login`, enter A's email, choose **Continue with email**, enter its password, and Continue.
2. Open Agents and a session; confirm A's resources load. Reload once to prove persistence.
3. Save the current refresh credential privately for the replay check, then choose **Sign out** in the sidebar user menu.
4. Check that the logout request succeeds and access, refresh, and organization cookies are cleared.
5. Open `/agents` directly, then use browser Back and reload. Protected content must not reload without authentication.
6. Use the saved refresh credential once at `POST /api/v1/auth/refresh`; inspect the response.
7. Sign in as B and inspect Agents and Sessions. Reload and inspect the organization switcher.

## Expected Result

- Sign-out returns to login and clears persisted organization selection and cached application data.
- The signed-out refresh credential returns 401; it cannot establish a new session.
- Protected navigation requires login. B sees only B's accessible data, never A's cached agents or sessions.
- This checks refresh revocation and browser cleanup. A separately copied access JWT remains valid until its short lifetime expires; immediate JWT revocation is not the contract.
