---
type: Test Case
title: "TC004: Ask User Text Question"
description: "Verify a free-form Text question renders a textarea, submits its string, and describes an unanswered deadline as skipping."
tags:
  - everruns
  - test-case
  - ui
  - ask-user
---
# TC004: Ask User Text Question

## Description

Verify the inline Ask User card handles an open question without inventing
choice options or an unattended answer.

## Preconditions

- Canonical local stack is running with `AUTH_MODE=none`.
- An Agent on a harness with `ask_user` enabled.
- A chat session with that Agent.

## Steps

1. Ask the Agent to ask an open question with `kind: "text"`, such as “What
   should I call this branch?”
2. Confirm the card shows one textarea with **Type your answer** and no radio
   inputs, option descriptions, or **Recommended** badge.
3. Confirm **Continue** stays disabled while the textarea is blank.
4. Wait for the deadline warning. Confirm it says the unanswered question will
   be **skipped** and does not say an option will be chosen.
5. Enter a distinctive string, press **Continue**, and confirm the collapsed
   card and the Agent's next message include the exact string.
6. Repeat with a credential-shaped string and confirm the card reports an error
   without echoing the submitted value.
7. Leave a fresh Text question unanswered through its deadline. Confirm the
   card resolves as declined and the Agent continues without an empty answer.

## Expected Result

- Text renders a textarea and no choice affordances.
- The string reaches the Agent unchanged through the free-text answer field.
- Credential-shaped text is rejected before persistence.
- Countdown and deadline resolution both skip the unanswered question.
