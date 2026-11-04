---
type: Test Case
title: "TC004: Sessions - Agent Failure Investigation"
description: "Verifies agent failure-count navigation, historical session filtering, outcome badges, and exact-turn Trace links."
tags: [everruns, test-case, ui, sessions, agents]
---

# TC004: Sessions - Agent Failure Investigation

## Description

Verify the investigation path from [Agents Home](../../../ui/agents-home.md) through the
agent's Sessions tab to the recorded terminal failure in Trace.

## Preconditions

- API, worker, UI and proxy are running with a session-authorized user or local `AUTH_MODE=none`.
- An agent has a session created more than 24 hours ago, with a failed turn in the last 24 hours.
- That session subsequently completed another turn successfully.
- Another session has a failed latest turn; another has only a recoverable tool error or cancellation.

## Test Data

| Field | Value |
| --- | --- |
| Desktop viewport | 1440 × 900 |
| Narrow viewport | 390 × 844 |
| Failure-only bucket | Turn started before the window and failed inside it |

## Steps

1. Open Agents. Hover and keyboard-focus the run chart.
2. Click the plain failure-count link and inspect its URL and selected agent tab.
3. Compare the failed-turn count and distinct-session count with the matching events.
4. Inspect the old recovered session and the session with a failed latest turn.
5. Click View failure and inspect the selected Trace turn, error step and banner.
6. Repeat with a failed turn that has no indexed step.
7. Switch to All sessions, back to With failed turns, and change the failure-time window.
8. Repeat the list and Trace navigation at the narrow viewport and in dark mode.

## Expected Result

- The list retains its original layout without column headers, arrows, inline timeframe or a
  separate latest-failure action. Tooltip says Last 24 hours and Each bar is 1 hour.
- The failure-count link selects the agent's Sessions tab and preserves the chart snapshot's bounds.
- Old, archived and recovered sessions are included. Cancellation and recoverable tool errors
  do not contribute failed turns. A zero-start bucket still draws its failure.
- The recovered session shows Completed; the latest failure shows Last turn failed. Running
  execution takes precedence over history, and cancellation shows Stopped.
- The error excerpt and timestamp belong to the latest failure in the selected window, not an
  earlier assistant reply or a newer run summary. Missing error detail is named explicitly.
- View failure resolves the terminal event to its exact Trace turn, selects an error step when
  available and shows the failed-turn banner without changing the session's current outcome.
- Filters, pagination, empty/loading/error states and narrow/dark layouts remain usable.
