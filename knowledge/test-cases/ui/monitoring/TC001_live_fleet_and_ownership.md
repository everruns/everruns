---
type: Test Case
title: "TC001: Monitoring - Live fleet and ownership"
description: "Verify aggregate execution, worker mapping, and actor diagnostics remain usable with thousands of sessions."
---
# TC001: Monitoring - Live fleet and ownership

## Description

Verify aggregate execution, worker mapping, and actor diagnostics remain usable with thousands of sessions.

## Preconditions

- Local stack with platform access (AUTH_MODE=none is acceptable only locally).
- A recorded fixture of at least 5,000 sessions across multiple organizations and 64 workers.
- Synthetic fixtures are inert; execution workers are drained before installing them.

## Test Data

| Input | Value |
|---|---|
| Population | Recorded local fixture IDs and API totals |
| Refresh deadline | Five seconds |

## Steps

1. Open Durable Execution → Monitoring and record fixture population totals and generated timestamp.
2. Observe two refreshes within five seconds; open Workers and choose a group.
3. Check the selected group's worker-to-org mapping; choose a holder.
4. Inspect an actor and compare holder, fence, expiry, renewal, and cursors with its API response.
5. Open Metrics; verify rates state their observation window.
6. Repeat Execution at 390px width and in the dark theme.

## Expected Result

Exact scoped totals; bounded worker groups and 200-row actor pages; ownership agrees with the API; no agent names or payloads; no page overflow or browser errors.
