---
type: Test Case
title: "TC004: Slack failure surfacing — no turn ends in silence"
description: "Verifies that a failed turn, an exhausted budget, and a turn producing no output each post exactly one terse notice with a session link, and that no internal error detail reaches a public channel."
tags:
  - everruns
  - test-case
  - ui
  - slack
---
# TC004: Slack failure surfacing — no turn ends in silence

## Description

Verifies the invariant that a Slack user never sends a message and receives
nothing: every terminal turn state that produces no reply produces a notice
instead.

Two things are being checked at once, and they pull against each other. The
notice must be informative enough to act on, and must not leak server-internal
error text into what is frequently a public channel. The session link is the
escape hatch that lets the notice stay terse.

## Preconditions

- A Slack app set up per [TC001](TC001_slack_app_setup.md).
- The tester can induce a turn failure — a deliberately broken tool, a revoked
  provider key, or an agent configured to fail.
- The tester can set a budget low enough to exhaust mid-turn.
- A frontend URL is configured, so notices carry a session link.

## Test Data

| Field | Value |
| --- | --- |
| Failure notice | `The agent could not finish this request.` |
| Cancellation notice | `This request was cancelled.` |
| No-output notice | `The agent finished without a reply.` |
| Link text | `View the session` |

## Steps

1. Induce a turn failure and message the bot in a channel thread.
2. Read the thread and click the link in the notice.
3. Set a budget that will be exhausted mid-turn, then send a prompt expensive
   enough to exhaust it.
4. Configure the agent to complete a turn without emitting a text message, then
   message the bot.
5. Send a normal prompt that succeeds, and count the messages in the thread.
6. Restart the server while a turn is cancelled but before it is observed, then
   inspect the thread and the delivery state.

## Expected Result

- Step 1: exactly one notice appears, matching the failure text in Test Data.
  It carries **no** stack trace, no provider error string, no internal
  identifier beyond the session link.
- Step 2: the link opens the session, where the real reason is visible to
  someone with access.
- Step 3: budget exhaustion surfaces as a notice, not silence.
- Step 4: the no-output case is distinguishable from a crash — the notice says
  the turn finished without a reply.
- Step 5: a successful turn posts its reply and **no** notice. One turn, one
  outcome, never both.
- Step 6: no delivery is re-registered for the already-cancelled turn, and no
  duplicate notice appears on restart.

## Notes

Step 5 matters as much as the failure steps. The first implementation of this
notice risked double-posting — a reply *and* a notice — which is a worse defect
than the silence it was fixing, because it makes the agent look broken when it
worked.

Step 6 guards a defect that was fixed once in the live path and initially
missed in the recovery path.
