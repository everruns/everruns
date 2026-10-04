---
type: Test Case
title: "TC004: Agent card channels"
description: "Verify compact channel summaries, responsive overflow, lifecycle indicators, and navigation from agent cards."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC004: Agent card channels

## Preconditions

- A DB-backed local stack runs with `AUTH_MODE=none`, or the tester has access to the organization.
- An active agent has five inbound channels with a mix of live, draft, and disabled states.
- A second active agent has no channels.

## Steps

1. Open `/agents` in grid view. Verify the populated agent has a compact Channels strip above
   the metadata footer, with transport icons, names, and state indicators.
2. At card widths of 500px, 360px, and 280px, verify three, two, and one channel chips appear,
   respectively. Their overflow controls read `+2`, `+3`, and `+4` without horizontal overflow.
3. Hover or keyboard-focus channel links. Verify accessible names and tooltips describe their
   live, draft, or paused state, without relying on color alone.
4. Activate a channel chip or overflow control. Verify it opens that agent's Integrations tab.
5. Verify the empty agent says **None configured**. Activate **Add** when management actions
   are available and verify it opens the agent's channel creation page.
6. Publish, unpublish, and disable a channel. Return to the collection and verify the card
   reflects its current state without a full browser reload.
7. Suspend all exposures for the populated agent. Verify its chips say **Suspended** and no
   longer display live indicators. Resume exposures and verify their original states return.
8. Archive the agent. Verify its channels remain visible but say **Agent unavailable** and no
   channel creation action appears.
9. Verify scheduled triggers do not appear in Channels.
10. Repeat the visual checks in list view and dark mode. Compare harness and example cards:
    headers, quiet tags, capability chips, and separated footers use the same card family.

## Expected Result

Channel summaries are current, readable at narrow widths, and scoped to the owning agent.
The collection loads summaries with its agent response, without a separate channel request
for each card. The summary never includes channel configuration, authentication, or secrets.
