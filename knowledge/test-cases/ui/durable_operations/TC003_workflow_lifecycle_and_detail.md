---
type: Test Case
title: "TC003: Workflow lifecycle and detail"
description: "Verify that the durable workflows UI filters executions, exposes event details and linked sessions, and safely cancels purpose-created test work."
tags:
  - everruns
  - test-case
  - ui
  - durable
---
# TC003: Workflow lifecycle and detail

## Description

Verify that the durable workflows UI filters executions, exposes event details and linked sessions, and safely cancels purpose-created test work.

## Preconditions

- The full stack is running and the user is signed in as an operator.
- Read-only list and detail checks may run on a shared stack.
- Cancellation approval runs only on an isolated local or test stack.
- Known completed and failed workflows exist with linked sessions and multiple event types.
- The cancellation target is a separate purpose-created, idempotent test workflow with no external side effects and a safely cancellable pending activity.

## Test Data

| Field | Value |
|---|---|
| List route | `/durable/workflows` |
| Detail route | `/durable/workflows/{workflowId}` |
| Search value | The test workflow type or ID |
| Terminal workflows | Known completed and failed workflows |
| Cancellation target | The purpose-created idempotent test workflow |

## Steps

1. Open `/durable/workflows` and verify the workflow count and status badges match the seeded executions.
2. Search for the test workflow, then filter by its status and confirm the list contains only matching rows.
3. Open **Tasks** and verify active and pending task metadata, including priority, attempt, claimant, schedule time, and workflow link.
4. Open **Dead Letter Queue** and verify failed activity, attempt, error, dead time, and requeue count fields.
5. Return to **Workflows** and open the known completed workflow.
6. Confirm the detail page shows completed status, timestamps, input, result, and an event history ordered by sequence.
7. Click **View Session** and confirm it opens the completed workflow's linked session transcript, then return.
8. Open the known failed workflow and confirm its detail page shows failed status, timestamps, input, error, and an event history ordered by sequence.
9. Return to **Workflows**, open the separate purpose-created cancellation target, click **Shutdown**, reject the prompt, and confirm the workflow remains running.
10. Click **Cancel**, reject the prompt, and confirm the workflow remains running.
11. On an isolated local or test stack only, reconfirm the selected workflow is the purpose-created idempotent test workflow, click **Cancel** again, approve the prompt, and refresh until the status and event history report cancellation.
12. Return to the list and confirm the canceled test workflow appears when the `cancelled` filter is selected.

## Expected Result

- Search and status filters expose only matching workflows.
- Task and dead-letter tabs show operational state and working workflow links.
- Workflow detail matches the selected ID and exposes input, terminal output, and event history.
- Session navigation preserves the workflow's linked session identity.
- Destructive actions require confirmation, and approved cancellation of the test workflow reaches both detail and list views.
- The case never approves cancellation on a shared stack or for a workflow it did not create.

## Cleanup

- Confirm the canceled test workflow has no running or pending activities.
- Remove any remaining test-only fixtures with the isolated stack's idempotent cleanup procedure.
