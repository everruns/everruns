---
type: Test Case
title: "TC004: Circuit breaker recovery controls"
description: "Verify that the circuit breaker UI reports open, half-open, and closed state and safely performs force-open, force-close, and reset actions."
tags:
  - everruns
  - test-case
  - ui
  - durable
---
# TC004: Circuit breaker recovery controls

## Description

Verify that the circuit breaker UI reports open, half-open, and closed state and safely performs force-open, force-close, and reset actions.

## Preconditions

- The full stack is running and the user is signed in as an operator.
- A closed circuit breaker exists with known success and failure counts.
- A known circuit breaker is in the half-open state.
- A disposable circuit breaker can be recreated by exercising its guarded operation.

## Test Data

| Field | Value |
|---|---|
| Route | `/durable/circuit-breakers` |
| Breaker | A disposable test breaker key |
| Half-open breaker | A known breaker key in the half-open state |

## Steps

1. Open `/durable/circuit-breakers` and locate the test breaker.
2. Confirm the Total, Open, and Closed summary counts agree with the grouped breaker cards.
3. Locate the known half-open breaker, confirm its badge reads `half open`, and verify its card appears under **Half-Open Circuit Breakers**.
4. Verify the test breaker card shows key, state, failure count, success count, last failure, opened time, and updated time.
5. Click **Force Open**, reject the dialog, and confirm the breaker remains closed.
6. Repeat **Force Open**, approve it, and confirm the card moves to **Open Circuit Breakers**, the badge reads `open`, and the Open summary increments.
7. Click **Force Close** and confirm the card returns to the closed group and the summaries update.
8. Click **Reset**, reject the dialog, and confirm the breaker remains.
9. Repeat **Reset**, approve it, and confirm the card disappears.
10. Exercise the guarded operation that owns the breaker and refresh until a new breaker card appears with reset counters.

## Expected Result

- Summary counts and open, half-open, and closed state groupings remain consistent.
- Force-open and reset actions require confirmation; canceling is non-destructive.
- Force-open blocks the breaker and force-close restores the closed state.
- Reset deletes stored breaker state, and normal guarded traffic recreates it.
