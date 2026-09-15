---
type: Test Case
title: "TC003: Slack channel thread — mention, markdown, attribution, history"
description: "Verifies channel-thread behaviour: an @mention starts one threaded session, markdown renders as blocks, multiple speakers are attributed, and joining mid-thread backfills prior history."
tags:
  - everruns
  - test-case
  - ui
  - slack
---
# TC003: Slack channel thread — mention, markdown, attribution, history

## Description

Verifies channel-thread behaviour, which is a different delivery path from the
assistant pane: discrete posts rather than streaming, threads rather than a
pane, and several humans rather than one.

The rendering assertions are the reason this is manual. The suite can prove we
sent a `markdown` block; only a human can confirm Slack renders the table.

## Preconditions

- A Slack app set up per [TC001](TC001_slack_app_setup.md).
- Session strategy `per_thread` (the default).
- The bot is in the test channel.
- A second human tester, for the attribution step.

## Test Data

| Field | Value |
| --- | --- |
| Markdown prompt | one whose answer contains a table, a heading, and a fenced code block |
| Long-output prompt | one whose answer exceeds Slack's per-block limit |

## Steps

1. `@mention` the bot in the channel with the markdown prompt.
2. Inspect the rendered reply.
3. Reply in the same thread with a follow-up.
4. Have the second tester reply in the same thread and address the bot.
5. In a **separate** thread with several existing human messages and no bot
   involvement, `@mention` the bot for the first time and ask it something that
   depends on what was said earlier in that thread.
6. Invite another user to the channel, generating a `channel_join` event.
7. `@mention` the bot with the long-output prompt.
8. Open the sessions for the two threads in the Everruns UI.

## Expected Result

- Step 2: the reply is threaded, not posted to the channel. The table renders
  as a table, the heading as a heading, and the code block with its language —
  not as raw markdown punctuation.
- Step 3: the follow-up lands in the same session.
- Step 4: the agent can tell the two speakers apart. Its answer reflects who
  said what.
- Step 5: the agent answers using the earlier thread history, which it only has
  because the thread was backfilled on first mention.
- Step 6: **no session is created and no turn runs.** A join message must not
  reach an agent.
- Step 7: the reply is split across blocks or messages, with no content
  silently dropped at the limit.
- Step 8: two threads produced two sessions; each carries its own routing tag
  and participants.

## Notes

Step 6 is a regression guard. `channel_join` arrives as `type: message` with
non-empty text and once created a session and burned a turn. It is cheap to
check and expensive to miss, because the symptom is spend rather than an error.
