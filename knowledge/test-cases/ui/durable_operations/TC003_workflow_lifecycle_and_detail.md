---
type: Test Case
title: "TC003: Workflow lifecycle and detail"
description: "Verify that the durable workflows UI filters executions, exposes event details and linked sessions, and cancels running work."
tags:
  - everruns
  - test-case
  - ui
  - durable
---
# TC003: Workflow lifecycle and detail

## Description

Verify that the durable workflows UI filters executions, exposes event details and linked sessions, and cancels running work.

## Preconditions

- The full stack is running and the user is signed in as an operator.
- Completed, failed, and running workflows exist.
- At least one workflow is linked to a session and has multiple event types.

## Test Data

| Field | Value |
|---|---|
| List route | `/durable/workflows` |
| Detail route | `/durable/workflows/{workflowId}` |
| Search value | A known workflow type or ID |

## Steps

1. Open `/durable/workflows` and verify the workflow count and status badges match the seeded executions.
2. Search for the known workflow, then filter by its status and confirm the list contains only matching rows.
3. Open **Tasks** and verify active and pending task metadata, including priority, attempt, claimant, schedule time, and workflow link.
4. Open **Dead Letter Queue** and verify failed activity, attempt, error, dead time, and requeue count fields.
5. Return to **Workflows** and open the known workflow.
6. Confirm the detail page shows status, timestamps, input, result or error, and an event history ordered by sequence.
7. For a linked workflow, click **View Session** and confirm it opens that session's transcript, then return.
8. For the running workflow, click **Shutdown**, reject the prompt, and confirm the workflow remains running.
9. Click **Cancel**, approve the prompt, and refresh until the status and event history report cancellation.
10. Return to the list and confirm the canceled workflow appears when the `cancelled` filter is selected.

## Expected Result

- Search and status filters expose only matching workflows.
- Task and dead-letter tabs show operational state and working workflow links.
- Workflow detail matches the selected ID and exposes input, terminal output, and event history.
- Session navigation preserves the workflow's linked session identity.
- Destructive actions require confirmation, and approved cancellation reaches both detail and list views.
