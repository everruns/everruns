---
type: Test Case
title: "TC003: Pause and Resume Schedule"
description: "Verify pausing prevents future automatic triggers, resuming restores them, and pause does not cancel executions already admitted."
tags:
  - everruns
  - test-case
  - ui
  - scheduled-tasks
---
# TC003: Pause and Resume Schedule

## Description

Verify pausing prevents future automatic triggers, resuming restores them, and pause does not cancel executions already admitted.

## Preconditions

- Disposable once-per-minute schedule with a working harmless target and platform-user access.
- Observe execution IDs and scheduled timestamps; allow for work already admitted before pause.

## Test Data

| Field | Value |
| --- | --- |
| Cron | `0 * * * * * *` |
| Observation windows | Two complete minute boundaries after each state change |

## Steps

1. Observe one successful automatic execution and record its ID.
2. Click Pause. Record the successful response time, then reload the detail.
3. Observe two complete minute boundaries and inspect executions' scheduled timestamps.
4. Click Resume, reload, and inspect the calculated next trigger.
5. Observe two complete minute boundaries and verify new executions are admitted.
6. Pause again and delete the disposable schedule after any admitted work settles.

## Expected Result

- Pause and Resume are direct mutations; no confirmation dialog is required.
- Pause persists as Paused and the UI presents no active next trigger. No future automatic execution is admitted while paused.
- Work admitted before the pause response may finish; do not misclassify that completion as a new trigger.
- Resume persists as Active, computes the next trigger, and admits new work at due boundaries.
