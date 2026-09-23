---
type: Test Case
title: "TC002: Ask User Nudge and Timeout"
description: "Verify the Ask User countdown appears before expiry naming the option that will be chosen, and that the turn resumes on its own with the declared default when the deadline passes."
tags:
  - everruns
  - test-case
  - ui
  - ask-user
---
# TC002: Ask User Nudge and Timeout

## Description

Verify the two deadline behaviors: the countdown that warns before a question
expires, and the automatic resolution that applies the declared default.

This is the case worth the wall-clock. The timing is the novel part of the
feature and the part most likely to regress silently — a countdown that never
appears, or a default applied without the user seeing it coming, both look fine
in unit tests.

## Preconditions

- Canonical local stack is running with `AUTH_MODE=none`.
- An Agent on a harness with `ask_user` enabled.

### Shortening the wall-clock

Ask the Agent to pass a short **`timeout_seconds`** on the `ask_user` call. Do
**not** reach for `TOOL_RESULT_TIMEOUT_SECS`: that governs the generic
stale-tool-result sweep, while an `ask_user` call carries its own `expires_at`,
which the sweep honours from the moment the session parks. Changing the
environment variable therefore does not move this deadline.

The server stamps both deadlines (`deadlines_for`, `crates/builtins/src/ask_user.rs`).
The nudge lead scales with the window rather than sitting at a fixed 60s, so a
short timeout still nudges after the question was asked:

| `timeout_seconds` | countdown appears at | expires at |
|---|---|---|
| 300 (default) | 4:00 | 5:00 |
| 120 | 1:36 | 2:00 |
| 60 | 0:48 | 1:00 |

`timeout_seconds` is validated to `1..=300`. Use **60** below.

The sweep runs every 30s, so automatic resolution lands at `expires_at` plus up
to one sweep interval. Allow 90s from the ask before calling case 2 a failure.

## Steps

1. Ask the Agent to put a single-select question to you with descriptions, a
   recommended default, and `timeout_seconds` of 60. Note the wall-clock time
   the card appears.
2. **Nudge.** Do not answer. Around 48s after the card appeared, confirm a
   countdown line appears in the card footer, styled as a warning with a clock
   icon, reading `Continuing with <label> in M:SS`.
   - Confirm `<label>` names the option that will actually be chosen — the
     question's declared default — and not merely the first option. The card
     falls back to the first option only when no default is declared, so
     declare one here: it is what makes this assertion meaningful, and it keeps
     the card and the server's resolution provably talking about the same
     option.
   - Confirm the countdown ticks down rather than rendering once and freezing.
   - Confirm **Continue** and **Decline** are still usable while it counts.
3. **Timeout.** Keep waiting. At 60s the deadline passes; the line becomes
   `Continuing with <label> now`.
4. Within 30s more, confirm without reloading that:
   - the card collapses on its own to `Auto-selected: <label>` with a check
     icon, and the inputs and buttons are gone;
   - the turn resumes by itself;
   - the Agent's next message reflects that the default was applied because
     nobody answered, rather than reading as though the user chose it.
5. **The countdown is not merely cosmetic.** Repeat step 1, and this time answer
   **before** the nudge. Confirm no countdown ever appears and the card settles
   to `Answered: <label>`.
6. Repeat step 1 once more and answer *during* the countdown, before expiry.
   Confirm the answer wins: the card shows `Answered: <label>` with your choice,
   never `Auto-selected`, and no second resolution arrives afterwards.

## Expected Result

- The countdown appears ahead of expiry, scaled to the window, and names the
  option that will be chosen.
- The countdown ticks, and is absent entirely when the question is answered
  before the nudge.
- At expiry the turn resumes on its own, with no reload and no user action.
- The resolved card is distinguishable from a human answer: `Auto-selected`,
  not `Answered`.
- The model's continuation reflects that no human answered.
- An answer given before expiry always beats the deadline, and resolves once.
