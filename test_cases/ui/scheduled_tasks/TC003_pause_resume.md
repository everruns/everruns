# TC003: Pause and Resume Schedule

## Description

Verify that pausing a schedule prevents automatic triggers, and resuming restores normal scheduling.

## Preconditions

- Schedule exists with a 1-minute cron expression
- Schedule is currently enabled

## Test Data

| Field | Value |
|-------|-------|
| Schedule Name | test-pause-resume |
| Cron Expression | * * * * * |
| Target Type | workflow |
| Workflow Type | echo |

## Steps

### Part A: Pause

1. Create schedule "test-pause-resume" with cron "* * * * *" (every minute)
2. Wait 1-2 minutes and verify automatic execution occurs
3. Note the current execution count
4. Click "Pause" button on the schedule
5. Confirm in dialog
6. Wait 2 minutes
7. Check execution count

### Part B: Resume

8. Click "Resume" button on the schedule
9. Confirm in dialog
10. Wait 2 minutes
11. Check execution count

## Expected Result

### Part A: Pause

- After clicking Pause, status changes to "Disabled" (gray badge)
- Next Run column shows "-" instead of a time
- No new executions appear during the 2-minute wait
- Execution count remains the same as noted in step 3

### Part B: Resume

- After clicking Resume, status changes to "Enabled" (green badge)
- Next Run column shows a time within 1 minute
- New executions appear after waiting
- Execution count increases by at least 1
