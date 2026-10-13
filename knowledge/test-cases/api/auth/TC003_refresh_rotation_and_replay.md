---
type: Test Case
title: "TC003: Auth - Refresh Rotation and Replay"
description: "Verify single-use refresh rotation, bounded concurrent consumption, logout revocation, and isolation between independent browser sessions."
tags:
  - everruns
  - test-case
  - api
  - auth
---
# TC003: Auth - Refresh Rotation and Replay

## Description

Verify single-use refresh rotation, bounded concurrent consumption, logout revocation, and isolation between independent browser sessions.

## Preconditions

- Canonical `AUTH_MODE=full` stack and a disposable verified account.
- Private cookie jars or an HTTP client that can retain one old refresh credential without printing it.
- At most two concurrent refresh requests, each with a 10-second timeout; disposable local fixtures only.

## Test Data

| Variant | Expected |
| --- | --- |
| Valid refresh | 200; new access and refresh credentials |
| Reuse consumed refresh | 401 |
| Two simultaneous uses of one refresh | Exactly one 200 and one 401 |
| Refresh after logout | 401 |
| Missing/invalid refresh | 401 |
| Independent second login session | Remains renewable after first session logs out |

## Steps

1. Log in and preserve its refresh credential privately. Renew once, then replay the old credential.
2. With a fresh login's refresh credential, send exactly two concurrent refresh requests. Record statuses without logging secrets.
3. Save the winning response's new cookie jar and renew again with its rotated refresh credential.
4. Create a second independent login session. Log out the first session using its current cookies and attempt its refresh again.
5. Renew the second session, then perform the missing/invalid variants with no fallback cookies.

## Expected Result

- Rotation consumes the old credential atomically; only one concurrent request can win.
- The winning rotated credential remains usable. Failed replay does not grant access or mint another valid session.
- Logout revokes that session's refresh and clears its browser auth cookies; an independent session is unaffected.
- This case does not assume immediate invalidation of already-issued access JWTs.

## Source Contract

[Refresh and logout handlers](../../../../crates/server/src/auth/routes/mod.rs).
