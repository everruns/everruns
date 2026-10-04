---
type: Test Case
title: "TC005: Pending Slack health"
description: "Verifies unresolved installation issues remain discoverable until current evidence verifies recovery."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC005: Pending Slack health

## Preconditions

- A canonical authenticated local stack with notifications enabled.
- An active agent with an installed Slack channel missing a required bot scope.
- Two users allowed to manage the agent; an additional denied or revoked user.
- Provider fixtures can return current scopes, unavailable evidence and mismatched identities.

## Steps

1. Wait for reconciliation. Open notifications; verify **Action required**, an unresolved count
   and a plain-language impact statement. Missing reaction permission must not imply replies fail.
2. Open the issue. Verify the announcement becomes read while the issue and warning remain visible
   in the bell, channel settings and **Settings → Health**.
3. Repeat the permission failure ten times. Verify one issue and one announcement for the episode.
4. Click **Remind me tomorrow**. Verify the issue remains open and the other user's state is unchanged.
   After the fixture advances reminder time, verify the existing announcement becomes unread once.
5. On a managed app, reconnect and decline Slack consent. Verify the issue remains open. Replay
   that callback; verify it fails. Repeat with a different returned app or workspace identity.
6. Approve the correct installation but return unavailable scope evidence. Verify health stays stale
   or needs a check. A redirect alone must not resolve it.
7. Return fresh required scopes for the expected workspace. Check again; verify the warning and
   unresolved count disappear, and the linked announcement records verified recovery.
8. Replace credentials during a slow check. Verify the old check cannot resolve the new configuration.
9. On a manual app, verify targeted add-scope/reinstall/token-update instructions and the existing
   channel editor link are shown.
10. Disable notifications. Verify the Health page and channel warnings remain available.
11. Revoke access and refresh list/detail/notifications or use an already-open stream. Verify health
    details and counts are no longer delivered. Cross-organization issue URLs must fail.
12. Disable, archive or delete the affected resource. Verify it is absent from active issues.
13. Repeat desktop and 390px mobile, light and dark; verify readable permission text and no overflow.

## Expected result

Reading and reminder state are personal. Recovery is shared and requires current trusted evidence.
Failure and incomplete consent leave a specific next step. Unknown evidence never says all healthy.
