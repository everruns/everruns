---
type: Test Case
title: "TC002: Monitoring - Access and isolation"
description: "Verify session readers can access their organization’s Monitoring and fleet access requires platform authority."
---
# TC002: Monitoring - Access and isolation

## Description

Verify session readers can access their organization’s Monitoring and fleet access requires platform authority.

## Preconditions

- Two organizations with distinct session fixtures.
- Full auth accounts: one tenant admin and one platform user; record membership and credential type.
- A tenant user whose session-read policy is denied.

## Test Data

| Input | Value |
|---|---|
| Population | Recorded local fixture IDs and API totals |
| Refresh deadline | Five seconds |

## Steps

1. Record each org fixture ID and population from an authorized fixture reader.
2. Open **Activity → Monitoring** as a session reader.
3. Compare the page totals and actor IDs to the organization fixtures.
4. Verify other-org fixtures, heartbeat, global worker load, and capacity are absent.
5. As the user without session-read permission, verify the page shows an access denial and organization endpoints return HTTP 403.
6. As the tenant admin, call each fleet endpoint and verify denial; as the platform user verify access.
7. Verify denied reads changed no actor ownership or session state.

## Expected Result

The session-read policy applies to every organization data request. Tenant scope does not disclose other organizations or global capacity. Fleet endpoints require platform-user status.
