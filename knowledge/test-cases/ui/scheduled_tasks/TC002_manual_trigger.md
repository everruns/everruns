---
type: Test Case
title: "TC002: Manual Trigger"
description: "Verify confirmation cancellation has no effect and an accepted manual trigger creates one observable execution of the configured workflow."
tags:
  - everruns
  - test-case
  - ui
  - scheduled-tasks
---
# TC002: Manual Trigger

## Description

Verify confirmation cancellation has no effect and an accepted manual trigger creates one observable execution of the configured workflow.

## Preconditions

- Disposable paused schedule from TC001; working registered target and worker.
- Caller has platform-user access. Record schedule ID, existing execution IDs, and input.

## Test Data

| Field | Value |
| --- | --- |
| Input | `{"message":"schedule-proof"}` |
| Completion budget | 60 seconds for the harmless target |

## Steps

1. Open the schedule detail and choose **Trigger**. Cancel the browser confirmation.
2. Refresh Execution History and verify no new execution or trigger request.
3. Choose Trigger again and accept. Record the returned execution ID and inspect history.
4. Use history Refresh until terminal status or the completion budget is reached.
5. Open View Workflow and verify the configured input and observed result.

## Expected Result

- Cancel performs no mutation. Accept performs one trigger request and produces one execution for this action.
- The execution may pass through pending/running too quickly to observe; a successful terminal record is sufficient.
- History records completed status, final duration in milliseconds, and a working workflow link; running duration may be absent.
- Workflow input matches the schedule. Timeout or failed status is a failure to investigate, not an implicit pass.
