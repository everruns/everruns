# TC001 Manage agent schedule triggers

## Description

Verify that the Agent detail Triggers tab supports the complete schedule-trigger lifecycle and displays recent outcomes.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An active agent is available

## Test Data

| Field | Value |
| --- | --- |
| Cron preset | Hourly :30 |
| Timezone | `America/Chicago` |
| Session mode | New session per run |
| Message | `Prepare the hourly report` |

## Steps

1. Navigate to the agent detail page and open the Triggers tab.
2. Click Add trigger, select Hourly :30, set the timezone and session mode, enter the message, and create the trigger.
3. Verify the list shows a human-readable cadence with timezone and does not expose raw cron.
4. Edit the trigger message and save it.
5. Disable the trigger, then enable it again.
6. Click Run now.
7. Verify a recent outcome appears with status and relative time.
8. Delete the trigger.

## Expected Result

- List, create, edit, enable/disable, run-now, and delete are available from the Triggers tab.
- Schedules and timezones are human-readable outside the editor.
- Recent durable execution outcomes are shown for the trigger.
- Deleting the trigger removes it from the list.
