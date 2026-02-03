# TC004: Max Concurrent Enforcement

## Description

Verify that the max_concurrent setting prevents overlapping executions when a previous execution is still running.

## Preconditions

- A workflow type exists that takes significant time to complete (e.g., has a sleep/delay)
- API server is running with scheduler enabled

## Test Data

| Field | Value |
|-------|-------|
| Schedule Name | slow-task |
| Cron Expression | * * * * * |
| Target Type | workflow |
| Workflow Type | slow_echo (or similar that takes >60s) |
| Max Concurrent | 1 |

## Steps

1. Create schedule "slow-task" with:
   - Cron: "* * * * *" (every minute)
   - Max Concurrent: 1
   - Target workflow that takes 2+ minutes to complete
2. Wait for first automatic execution to start
3. Verify execution status shows "Running"
4. Wait for the next cron trigger time (1 minute)
5. Check if new execution was created
6. Wait for first execution to complete
7. Wait for next cron trigger
8. Check if new execution starts

## Expected Result

- First execution starts and shows "Running"
- At the 1-minute mark, no new execution is created (due to max_concurrent=1)
- Schedule's "Next Run" may show the skipped trigger or advance to next slot
- After first execution completes, the next trigger creates a new execution
- At no point are there 2 concurrent "Running" executions for this schedule

## Alternative API Verification

```bash
# While first execution is running:
curl "$API_URL/v1/durable/schedules/$SCHEDULE_ID" | jq '.stats'
# Should show only 1 running

# Try manual trigger while execution running:
curl -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger"
# Should return error or skip message
```
