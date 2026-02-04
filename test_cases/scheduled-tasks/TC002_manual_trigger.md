# TC002: Manual Trigger

## Description

Verify that a schedule can be manually triggered and creates an execution that runs the target workflow.

## Preconditions

- Schedule "ping-service" exists and is enabled (from TC001)
- The target workflow type is registered and functional

## Test Data

| Field | Value |
|-------|-------|
| Schedule Name | ping-service |

## Steps

1. Navigate to Durable > Schedules page
2. Find "ping-service" in the list
3. Click on the schedule row to open detail page
4. Click "Trigger Now" button in the header
5. Confirm in the confirmation dialog
6. Observe the Executions table

## Expected Result

- Confirmation dialog appears asking "Trigger ping-service now?"
- After confirmation, success notification appears
- New execution appears at top of Executions table
- Execution shows:
  - Scheduled At: current time
  - Status: "Running" (yellow badge)
  - Duration: incrementing counter
- After a few seconds, status changes to "Completed" (green badge)
- Workflow ID column is populated with a link
- Clicking the Workflow ID link navigates to the workflow detail page
- Workflow detail shows the input `{"message": "health check"}`
