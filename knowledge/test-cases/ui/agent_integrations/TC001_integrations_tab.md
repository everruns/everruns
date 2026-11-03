---
type: Test Case
title: "TC001: Agent Channels and triggers tab"
description: "Verifies the Agent detail Channels and triggers tab lists channels and triggers in one table style, each row opening its own page, with GitHub and Pause agent elsewhere."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC001: Agent Channels and triggers tab

## Description

Verifies the Agent detail Channels and triggers tab lists channels and triggers in one table style, each row opening its own page, with GitHub and Pause agent elsewhere.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An agent exists with at least one channel in `draft` and at least one schedule trigger

## Test Data

| Field | Value |
| --- | --- |
| Agent name | Dad Joke Agent |
| Channel type | api_channel |

## Steps

1. Navigate to `/agents` and open the agent's detail page.
2. Verify the tab rail shows **Channels and triggers** and shows neither **Integrations**, **Triggers** nor **Integrate**.
3. Open the tab. Verify the address is `/agents/{agentId}?tab=integrations`, then reload and confirm it is still selected.
4. Verify the **Channels** table lists the channel with columns Channel, Status (`Draft`) and Last used.
5. Open the row menu and choose **Publish**. Verify the status becomes `Published` without a reload.
6. Click the row. Verify it opens `/agents/{agentId}/channels/{channelId}`; go back.
7. Verify the **Triggers** table renders below Channels in the same style, with columns Trigger, Status and Last run. The schedule row shows a human-readable cadence, not a raw cron expression.
8. Click the trigger row. Verify it opens `/agents/{agentId}/triggers/{triggerId}` with a Set up card and Recent runs; go back.
9. Verify the tab has no GitHub section and no **Accept traffic** switch.
10. Open **Add channel**. Verify Webhook is not offered and a link offers **Add a webhook trigger**; follow it and verify the Webhook trigger form opens with a generated token.
11. On the Agent tab, verify the More list has a **GitHub** row; click it and verify it opens `/agents/{agentId}/github`.
12. Open the header **⋯** menu and choose **Pause agent**, then confirm. Verify a **Paused** badge and a "This agent is paused" notice with **Resume** appear.
13. Click **Resume**. Verify the badge and notice disappear and the channel is still `Published`.
14. Repeat at a narrow mobile width in light and dark modes. Verify the tables collapse to one column per row and the page has no horizontal overflow.

## Expected Result

- One tab named Channels and triggers; channels and triggers share one table style, and every row opens a page of its own.
- The Publish action publishes and unpublishes a single channel without affecting its siblings.
- No raw cron expression appears outside an editable Cron input.
- Webhooks are created as triggers, not channels.
- GitHub lives on its own page, linked from the Agent tab.
- Pause agent stops the agent and Resume restores each channel's and trigger's own state.
