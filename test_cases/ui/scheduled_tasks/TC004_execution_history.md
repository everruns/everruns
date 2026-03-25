# TC004: Execution History

## Description

Verify that execution history is displayed correctly with proper sorting, status indicators, and navigation.

## Preconditions

- Schedule exists with multiple past executions (at least 5)
- Executions include both successful and failed runs

## Test Data

| Field | Value |
|-------|-------|
| Schedule Name | history-test |
| Min Executions | 5 |

## Steps

1. Navigate to schedule detail page for a schedule with multiple executions
2. Observe the Executions table
3. Verify sorting order
4. Examine a completed execution row
5. Examine a failed execution row (if present)
6. Click on a Workflow ID link
7. Return to schedule detail
8. Use pagination if more than 10 executions exist

## Expected Result

### Table Display

- Executions are sorted by most recent first (descending by scheduled_at)
- Each row shows:
  - Scheduled At: formatted datetime
  - Started At: formatted datetime
  - Duration: formatted as "Xs" or "Xm Ys"
  - Status: badge with appropriate color
    - Running: yellow
    - Completed: green
    - Failed: red
    - Skipped: gray
  - Workflow ID: clickable link (or Task ID for activity targets)
  - Error: truncated error message for failed executions

### Status-Specific Behavior

- Completed: Shows green badge, duration is final, has workflow link
- Failed: Shows red badge, error column populated, may or may not have workflow link
- Running: Shows yellow badge, duration is live/incrementing
- Skipped: Shows gray badge, no workflow link, reason in error column

### Navigation

- Clicking Workflow ID navigates to `/durable/workflows/{id}`
- Workflow detail page shows the execution's input and output
- Back button returns to schedule detail

### Pagination

- If >10 executions, pagination controls appear
- "Next" loads older executions
- "Previous" loads newer executions
- Page indicator shows "Page X of Y"
