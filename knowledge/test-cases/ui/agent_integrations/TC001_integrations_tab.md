---
type: Test Case
title: "TC001: Agent Integrations tab"
description: "Verifies the Agent detail Integrations tab lists the agent's channels and triggers, publishes a channel from its row, and replaces the separate Triggers and Integrate tabs."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC001: Agent Integrations tab

## Description

Verifies the Agent detail Integrations tab lists the agent's channels and triggers, publishes a channel from its row, and replaces the separate Triggers and Integrate tabs.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An agent exists with at least one channel in `draft`

## Test Data

| Field | Value |
| --- | --- |
| Agent name | Dad Joke Agent |
| Channel type | api_endpoint |

## Steps

1. Navigate to `/agents` and open the agent's detail page.
2. Verify the tab rail shows **Integrations** and shows neither **Triggers** nor **Integrate**.
3. Open the Integrations tab.
4. Verify the stat strip shows Health, Invocations 24h, Success rate, and Activity, and that Health counts live channels rather than enabled ones.
5. Verify the **Channels** section lists the channel, with a status badge reading `draft`.
6. Toggle the row's **Live** switch on.
7. Verify the badge becomes `live` and the subline reads "Live" without the page being reloaded.
8. Click the row to expand it.
9. Verify the expanded panel shows a **Use it** block whose URL contains the channel's own id (`/v1/e/{channelId}/…`), not a placeholder.
10. Verify the expanded row's Configure link points at `/agents/{agentId}/channels/{channelId}`.
11. Verify the **Triggers** section renders below Channels, and that any schedule row shows a human-readable cadence rather than a raw cron expression.
12. In the rail, toggle **Suspend all** on.
13. Verify Health reads "Suspended".

## Expected Result

- One tab named Integrations; no Integrate tab and no separate Triggers tab.
- Channel rows show the channel's own lifecycle, not the owning App's publish state.
- The Live switch publishes and unpublishes a single channel without affecting its siblings.
- The expanded row's snippet carries that channel's real URL.
- No raw cron expression appears outside an editable Cron input.
- Suspending exposures is reflected in the stat strip.
