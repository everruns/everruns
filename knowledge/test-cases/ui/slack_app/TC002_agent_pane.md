---
type: Test Case
title: "TC002: Slack agent pane — streaming, status, title, stop"
description: "Verifies the assistant pane streams replies progressively, shows a status line while a tool runs without leaking tool names, sets the thread title, and cancels the turn from the stop button."
tags:
  - everruns
  - test-case
  - ui
  - slack
---
# TC002: Slack agent pane — streaming, status, title, stop

## Description

Verifies the four agent-surface behaviours in Slack's assistant pane:
progressive streaming, a status line while a tool runs, the thread title, and
the stop button.

Every assertion here is about how Slack *renders* something. The integration
suite can prove we called `chat.appendStream`; only a human looking at the pane
can prove the reply appears progressively rather than arriving whole. That gap
is the reason this case exists.

## Preconditions

- A Slack app set up per [TC001](TC001_slack_app_setup.md), with the agent
  surface enabled and re-installed (`assistant:write` is a new scope, so an app
  installed before the surface was enabled will not show a pane).
- The bound agent has at least one tool and takes long enough to observe —
  tens of seconds, not two. An agent that answers instantly makes streaming and
  status unobservable.
- Tool activity visibility is at its default (`Generic`).

## Test Data

| Field | Value |
| --- | --- |
| Prompt | one that provokes a tool call and a long answer |
| Expected generic status | the configured generic tool text (default `Working...`) |
| Long-answer prompt | something producing several paragraphs |

## Steps

1. Open the Everruns agent from Slack's top navigation. The assistant pane
   opens in split view.
2. Send the prompt from Test Data.
3. Watch the pane continuously from send until the reply completes. Do not look
   away: the assertion is about the transition, not the end state.
4. Note what the pane shows while a tool is running, before any answer text.
5. Note the thread title in the pane once the turn is under way.
6. Send the long-answer prompt. Partway through the reply streaming, press
   **stop**.
7. Send a further message in the same pane thread.
8. Open the corresponding session in the Everruns UI.

## Expected Result

- Step 3: text appears progressively. A reply that appears all at once is a
  failure even if its content is correct.
- Step 4: a status line is visible. It shows the generic tool text and **must
  not** contain the tool's real name, its arguments, its result, or an internal
  call id. Leaking any of those is a failure regardless of how useful it looks.
- Step 5: the thread is titled from the session title, not left as Slack's
  default.
- Step 6: streaming stops promptly, the stream is closed rather than left
  spinning, and the thread shows `This request was cancelled.` with a **View
  the session** link.
- Step 7: the reply lands in the same session — the pane thread is one session
  across turns.
- Step 8: the session shows the cancelled turn, and its transcript matches what
  the pane displayed.

## Notes

Step 6 is the one most worth doing carefully. A stream that is never closed
leaves a Slack message spinning forever, which is worse than the silence it
replaced, and a stuck spinner is only visible to a human.
