---
type: Test Case
title: "TC001: Observer lifecycle and quality"
description: "Verify that an enabled observer can be created, validated, inspected, paused, resumed, edited, scored, and archived."
tags:
  - everruns
  - test-case
  - ui
  - observers
---
# TC001: Observer lifecycle and quality

## Description

Verify that an enabled observer can be created, validated, inspected, paused, resumed, edited, scored, and archived.

## Preconditions

- The full stack is running and the user is signed in.
- A dedicated test organization has no other active users, and its `observers` feature is enabled.
- The original `observers` feature state is recorded before the test.
- A test agent and harness exist.
- The configured judge model can score a completed production turn.

## Test Data

| Field | Value |
|---|---|
| List route | `/observers` |
| Name | `Manual quality observer` |
| Session tag | `eve-995-observer` |
| Sampling rate | `100%` |
| Scorer | `answer_completeness` |

## Steps

1. Open `/observers` and click **New Observer**.
2. Submit without a name or scorer and verify `Name is required` and `Add at least one scorer.` appear.
3. Enter the test name and description, set sampling to 100%, select the test agent and harness, and add the session tag.
4. Add the `answer_completeness` catalog scorer and create the observer.
5. Confirm navigation to `/observers/{observerId}` and verify the active badge, match rules, selected agent and harness, tag, sampling rate, and scorer summary.
6. Click **Pause**, verify the badge becomes `paused`, then click **Resume** and verify it returns to `active`.
7. Click **Edit**, change the description and sampling rate, save, and verify the detail page reflects both changes.
8. Run a matching tagged production turn and return to the observer's **Quality** tab.
9. Confirm a recent score appears with scorer, value, pass/fail, status, reasoning, time, and a working session link.
10. Click **Archive** and confirm navigation returns to `/observers` and the archived observer is absent from the active list.
11. Disable the feature for the organization and verify `/observers`, `/observers/new`, and the saved edit URL show the disabled state instead of observer data.

## Expected Result

- Create validation blocks incomplete observer definitions.
- Create and edit persist match rules, sampling, scorers, name, and description.
- Pause and resume update lifecycle state without changing configuration.
- Matching production work produces a navigable quality score.
- Archive removes the observer from the active list.
- A disabled feature does not expose list, create, or edit content.

## Cleanup

- Restore the test organization's `observers` feature to its recorded original state and verify `/observers` returns to that state.
