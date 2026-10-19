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
| Channel type | api_channel |

## Steps

1. Navigate to `/agents` and open the agent's detail page.
2. Verify the tab rail shows **Integrations** and shows neither **Triggers** nor **Integrate**.
3. Open the Integrations tab. Verify the address is `/agents/{agentId}?tab=integrations`, then reload and confirm Integrations is still selected.
4. Verify the stat strip shows Health, Invocations 24h, Success rate, and Activity, and that Health counts live channels rather than enabled ones. Unavailable activity metrics use a flat baseline with an explanatory caption.
5. Verify the **Channels** section lists the channel, with a status badge reading `draft`.
6. Toggle the row's **Publish** switch on.
7. Verify the badge becomes `live` and the subline reads "Live" without the page being reloaded.
8. Click the row to expand it.
9. Verify the expanded panel shows a **Use it** block whose URL contains the channel's own id (`/v1/channels/{channelId}/…`), not a placeholder.
10. Verify the expanded row's Configure link points at `/agents/{agentId}/channels/{channelId}`.
11. Verify a single **Triggers** heading renders below Channels, with **Add trigger** beside it and compact GitHub setup beneath it. Any schedule row shows a human-readable cadence rather than a raw cron expression.
12. In the **All channels** rail, turn **Enabled** off.
13. Verify Health reads "Disabled" and says publish settings are kept.
14. Repeat at a narrow mobile width in light and dark modes. Verify the sections stack, channel actions remain readable, and the page has no horizontal overflow.

## Expected Result

- One tab named Integrations; no Integrate tab and no separate Triggers tab.
- Channel rows show the channel's own lifecycle, not the owning App's publish state.
- The Publish switch publishes and unpublishes a single channel without affecting its siblings.
- The expanded row's snippet carries that channel's real URL.
- No raw cron expression appears outside an editable Cron input.
- Turning Enabled off pauses every channel and keeps each channel's publish state. The stat strip shows Disabled.
