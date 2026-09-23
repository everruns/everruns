---
type: Test Case
title: "TC003: Ask User Supersede and Reload"
description: "Verify that answering in the message box cancels the pending Ask User card and delivers the message, and that pending and completed cards survive a reload."
tags:
  - everruns
  - test-case
  - ui
  - ask-user
---
# TC003: Ask User Supersede and Reload

## Description

Verify the chat-supersedes-question rule and that card state is served from the
session rather than held only in the browser.

## Preconditions

- Canonical local stack is running with `AUTH_MODE=none`.
- An Agent on a harness with `ask_user` enabled.
- Use a long enough `timeout_seconds` (the 300s default is fine) that no
  deadline fires mid-case.

## Steps

1. **Chat supersedes.** Get the Agent to put a question to you. Instead of
   using the card, type an answer in the message box and send it.
   - Confirm the card resolves as cancelled, collapsing to
     **Questions cancelled** with an X icon and no buttons.
   - Confirm the typed message is delivered as a normal user message and the
     Agent acts on it.
   - Confirm the turn resumes exactly once — no duplicate continuation from the
     cancelled question.
2. **Reload mid-question.** Get the Agent to put another question to you and do
   not answer. Reload the page.
   - Confirm the card comes back **pending**, with its questions, descriptions,
     **Recommended** badge, and working **Continue** / **Decline** buttons.
   - Confirm any selection made before the reload is not expected to survive it,
     but the question itself does.
   - Answer it after the reload and confirm the turn resumes normally.
3. **Reload after answering.** With that card now completed, reload again.
   - Confirm it renders in its completed state — `Answered: <label>` — with no
     inputs and no buttons, so a past question cannot be answered twice.
4. Repeat step 3 for a declined card (**Questions declined**) and a cancelled
   card (**Questions cancelled**) and confirm both survive a reload in their
   terminal state.
5. **No double submit.** On a fresh pending card, press **Continue** and, while
   it reads *Submitting…*, confirm the buttons are disabled so a second submit
   cannot be issued.

## Expected Result

- A typed message while a question is pending cancels the question and is
  delivered as a message; the turn resumes once.
- A pending card survives reload and stays answerable.
- A resolved card survives reload in its terminal state, with no inputs or
  buttons, for all of answered, declined, cancelled, and auto-selected.
- Submission is single-shot: controls disable while it is in flight.
