---
type: Test Case
title: "TC001: Ask User Question Kinds"
description: "Verify single-select, multi-select, free-text other, and multi-question Ask User cards render their options and deliver every chosen value to the model."
tags:
  - everruns
  - test-case
  - ui
  - ask-user
---
# TC001: Ask User Question Kinds

## Description

Verify that the inline Ask User card renders each question shape correctly and
that every chosen value reaches the model when the turn resumes.

## Preconditions

- Canonical local stack is running with `AUTH_MODE=none`.
- An Agent on a harness with `ask_user` enabled (the generic and platform chat
  harnesses both have it).
- A chat session with that Agent.

Drive each case by asking the Agent to put the question to you rather than by
calling the tool directly, so the card, the resume, and the model's use of the
answer are all exercised in one pass.

## Steps

1. **Single-select.** Ask the Agent something that forces one choice between
   described alternatives (for example: "Ask me which database to use — Postgres
   or SQLite — and recommend one. Give each option a description.").
   - Confirm the card header reads **The agent needs your input** above
     *Choose an answer to continue the conversation.*
   - Confirm the options are **radio** inputs, each showing its description, and
     that the recommended option carries a **Recommended** badge.
   - Confirm **Continue** is disabled until an option is selected.
2. Select the non-recommended option and press **Continue**. Confirm the card
   collapses to `Answered: <label>` with a check icon, and that the Agent's next
   message uses the option you picked, not the recommended one.
3. **Multi-select.** Ask for a question with several simultaneously valid answers
   (for example: "Ask me which of these regions to deploy to, multi-select.").
   - Confirm the options are **checkbox** inputs.
   - Select at least two, press **Continue**, and confirm the collapsed card and
     the Agent's next message both name **every** value you selected.
4. **Other.** Ask a question that allows a free-text answer.
   - Confirm an **Other** choice is offered, and that the
     *Type another answer* text input is hidden until **Other** is selected
     rather than always shown.
   - Enter a distinctive sentinel phrase, submit, and confirm the sentinel
     reaches the Agent's next message verbatim.
5. Ask a question that does **not** allow free text and confirm no such input is
   offered.
6. **Multi-question.** Ask the Agent to put two questions in one call. Confirm
   both render in a single card with one **Continue** button, that **Continue**
   stays disabled until both are answered, and that one submit delivers both
   answers.
7. Press **Decline** on a fresh card. Confirm it collapses to
   **Questions declined** with an X icon and the turn resumes without an answer.

## Expected Result

- Single-select renders radios; multi-select renders checkboxes.
- Option descriptions and the **Recommended** badge render.
- **Continue** is disabled until every question in the card is answered.
- Every selected value, including free text, reaches the model verbatim.
- A completed card shows a terminal summary and no buttons: `Answered: <labels>`
  for a submitted answer, **Questions declined** for a decline.
