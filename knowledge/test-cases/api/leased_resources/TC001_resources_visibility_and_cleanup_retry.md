---
type: Test Case
title: "TC001: Leased Resources - Persistence and Cleanup Retry"
description: "Verify provider leases persist across stack restart and cleanup failures retry through the durable worker without exposing provider secrets in metadata."
tags:
  - everruns
  - test-case
  - api
  - leased-resources
---
# TC001: Leased Resources - Persistence and Cleanup Retry

## Description

Verify provider leases persist across stack restart and cleanup failures retry through the durable worker without exposing provider secrets in metadata.

## Preconditions

- Canonical persistent local stack with worker and durable scheduler; no shared production database.
- Controlled Browserless fixture and a real model/capability run that creates a disposable browser lease.
- Private test tooling can inspect the exact lease row and force one controlled cleanup failure, then restore success.

## Test Data

| Field | Value |
| --- | --- |
| Provider/type | Browserless / browser_session |
| Correlation | Exact returned session, lease, and external resource IDs |
| Recovery budget | Configured lease expiry, retry deadline, and scheduler poll interval plus 30 seconds |

## Steps

1. Run the disposable browser-capable agent and record its actual lease ID. Inspect its stored non-secret metadata.
2. Restart only this prefix's stack without resetting data. Verify the same lease/session survives.
3. End or expire the disposable lease and make the controlled cleanup endpoint fail once.
4. Inspect the exact lease's cleanup-failed state and retry deadline, plus the leased-resource-cleanup schedule execution/log outcome.
5. Restore provider cleanup and wait within the stated recovery budget; verify the same lease becomes released and its provider resource is gone.
6. Confirm another active unexpired lease is unchanged, and captured metadata/logs contain no credentials.

## Expected Result

- Persistence retains the same lease across restart; it is not fabricated by inserting a row into an unrelated resource registry.
- Failed cleanup records a retry rather than marking the resource released. A later successful attempt releases exactly the correlated resource.
- Unexpired leases remain active, and provider credentials are absent from public metadata/evidence.
- Session resource registry rows and provider lease rows are separate contracts. `/sessions/<id>/resources` and a removed Resources tab do not prove lease persistence or cleanup.

## Source Contract

[Lease service](../../../../crates/server/src/domains/session_resources/leased_resource.rs) and [cleanup worker](../../../../crates/worker/src/leased_resource_cleanup.rs).
