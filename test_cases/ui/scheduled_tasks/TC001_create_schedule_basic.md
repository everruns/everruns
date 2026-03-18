# TC001: Create Schedule - Basic Creation

## Description

Verify that a scheduled task can be created with required fields and appears in the schedules list.

## Preconditions

- API server is running in full mode (PostgreSQL)
- At least one workflow type is registered (e.g., `echo` or `health_check`)
- User has access to Durable section in UI

## Test Data

| Field | Value |
|-------|-------|
| Name | ping-service |
| Description | Health check every 30 minutes |
| Cron Expression | */30 * * * * |
| Timezone | UTC |
| Target Type | workflow |
| Workflow Type | echo |
| Input | `{"message": "health check"}` |
| Enabled | true |

## Steps

1. Navigate to Durable > Schedules page
2. Click "Create Schedule" button
3. Enter Name: "ping-service"
4. Enter Description: "Health check every 30 minutes"
5. Enter Cron Expression: "*/30 * * * *"
6. Verify cron preview shows "Every 30 minutes"
7. Keep Timezone as "UTC"
8. Select Target Type: "Workflow"
9. Select Workflow Type: "echo"
10. Enter Input JSON: `{"message": "health check"}`
11. Ensure "Enabled" is checked
12. Click "Create" button

## Expected Result

- Success notification appears
- Schedule appears in the schedules list
- Status column shows "Enabled" (green badge)
- Cron column shows "*/30 * * * *"
- Next Run column shows a time within 30 minutes from now
- Target column shows "workflow: echo"
- Last Run column shows "-" (never run)
