---
type: Test Case
title: "TC001: Cross-agent Exposures view"
description: "Verifies the Exposures view lists every exposure in the org, resolves state against the agent-level terms, and filters to exactly the publicly reachable set."
tags:
  - everruns
  - test-case
  - ui
  - exposures
---
# TC001: Cross-agent Exposures view

## Description

Verifies the Exposures view lists every exposure in the org, resolves state against the agent-level terms, and filters to exactly the publicly reachable set.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- The org is seeded with a mix of states, not an empty list

## Test Data

Seed at least these five exposures across at least three agents:

| Exposure | Config | Agent state | Expected row |
| --- | --- | --- | --- |
| AG-UI | anonymous, published | active | Live · Anonymous |
| AG-UI | anonymous, unpublished | active | Draft · Anonymous (not live) |
| Webhook | token, published | active | Live · Authenticated |
| AG-UI | anonymous, published | exposures suspended | Suspended · Anonymous (not live) |
| Any | published | archived agent | Agent inactive |

## Steps

1. Navigate to **Exposures** in the Operational section of the sidebar.
2. Verify every exposure above appears, regardless of owning agent.
3. Verify the stat strip counts Live, Publicly reachable, Suspended agents, and Transports.
4. Verify the live anonymous row sorts first and is visually distinct (tinted row, red Anonymous with a globe).
5. Verify the suspended agent's anonymous channel reads **Suspended**, not Live, and its Access reads **Anonymous (not live)**.
6. Verify the archived agent's channel reads **Agent inactive**, not Live.
7. Set the state filter to **Publicly reachable**.
8. Verify exactly the live-and-anonymous exposures remain — the draft anonymous and suspended anonymous ones are excluded.
9. Reset the filter. Click **Suspend** on a live row.
10. Verify the row's state becomes Suspended and its button becomes Resume without a page reload.
11. Open that agent's Integrations tab and verify the suspend switch is on.
12. Click **Resume** and verify both surfaces return to their prior state.
13. Click an agent name and verify it opens that agent's Integrations tab.

## Expected Result

- Every channel and trigger in the org appears, across all agents.
- State never reads Live for something the server would refuse: the agent-level suspend and archived cases both override the channel's own status.
- "Publicly reachable" returns exactly the live-and-anonymous set.
- Anonymous configuration is visible before it becomes reachable, so resuming an agent holds no surprise.
- Suspending from this view takes effect immediately and is reflected on the agent page.
