---
type: Test Case
title: "TC002: Monitoring - Adoption and isolation"
description: "Verify org adoption controls org Monitoring and tenant readers cannot access the fleet."
---
# TC002: Monitoring - Adoption and isolation

## Description

Verify org adoption controls org Monitoring and tenant readers cannot access the fleet.

## Preconditions

- Two organizations with distinct session fixtures.
- Full auth accounts: one tenant admin and one platform user; record membership and credential type.
- Org adoption is disabled initially; use the local anonymous admin only for the UI flag-toggle variant.

## Test Data

| Input | Value |
|---|---|
| Population | Recorded local fixture IDs and API totals |
| Refresh deadline | Five seconds |

## Steps

1. Record each org fixture ID and population from an authorized fixture reader.
2. Open org Monitoring with adoption disabled; verify navigation is hidden and the direct page cannot load data.
3. Enable Monitoring in Settings → Features; open the org page and compare its totals and actor IDs.
4. Verify other-org fixtures, heartbeat, global worker load, and capacity are absent.
5. Disable adoption while the page remains open; within five seconds verify cached population disappears.
6. As the tenant admin, call each fleet endpoint and verify denial; as the platform user verify access.
7. Verify denied reads changed no actor ownership or session state.

## Expected Result

Adoption gates every org data request; existing session-read policy also applies. Tenant scope does not disclose other organizations or global capacity. Fleet endpoints require platform-user status.
