---
type: Test Case
title: "TC001: Create Schedule - Creation and Validation"
description: "Verify durable schedule creation, required fields, invalid JSON recovery, and persisted target and enabled state."
tags:
  - everruns
  - test-case
  - ui
  - scheduled-tasks
---
# TC001: Create Schedule - Creation and Validation

## Description

Verify durable schedule creation, required fields, invalid JSON recovery, and persisted target and enabled state.

## Preconditions

- Canonical persistent stack; caller has platform-user access and worker is running.
- A harmless registered workflow target is available; use its actual name, not an assumed `echo` registration.

## Test Data

| Field | Value |
| --- | --- |
| Name | `manual-schedule-<run-id>` |
| Description | Manual creation proof |
| Cron | `0 */30 * * * * *` |
| Target type | Workflow |
| Target name | Actual registered harmless workflow |
| Input | `{"message":"schedule-proof"}` |

## Steps

1. Open `/durable/schedules` and Create Schedule. Try submitting while required fields are empty; no create request may occur.
2. Enter the name, description, and cron. Select Workflow and enter the target name.
3. Enter malformed input JSON and create; inspect the inline error and Network.
4. Correct the input, turn **Enable schedule immediately** off, and create.
5. Open the resulting detail and reload. Inspect the create payload and saved target.
6. Resume the schedule and inspect the next trigger, then pause it for cleanup.

## Expected Result

- Malformed JSON shows an error without a request; correction succeeds without reopening the dialog.
- Creation persists the exact target type, name, input, and disabled state. The dialog closes and the list contains the returned ID.
- The paused detail says Paused; Resume changes it to Active and calculates a next trigger.
- Current UI uses a target-name input and seven-field cron help; it does not expose a timezone picker or natural-language cron preview.

## Cleanup

Delete only the disposable schedule.
