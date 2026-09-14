---
type: Test Case
title: "TC001: Queue and dead-letter recovery"
description: "Verify that the durable queue UI shows live task state, filters tasks, enqueues work, and safely recovers or removes dead-letter entries."
tags:
  - everruns
  - test-case
  - ui
  - durable
---
# TC001: Queue and dead-letter recovery

## Description

Verify that the durable queue UI shows live task state, filters tasks, enqueues work, and safely recovers or removes dead-letter entries.

## Preconditions

- The full stack is running and the user is signed in as an operator.
- At least one worker is active.
- The queue contains pending, claimed, completed, and failed tasks from at least two activity types.
- The dead-letter queue contains one recoverable entry and one disposable entry.

## Test Data

| Field | Value |
|---|---|
| Route | `/durable/queues` |
| Search value | A known activity ID |
| Status filter | `failed` |
| Recoverable entry | A task that succeeds when retried |

## Steps

1. Open `/durable/queues` and compare the Pending, Processing, Completed/hr, and Failed/hr totals with the seeded task state.
2. Confirm the oldest-pending, average-wait, and average-execution values are present.
3. Open **Tasks**, select the `failed` status, and confirm only failed tasks remain.
4. Search for the known activity ID, then clear the search and status filters.
5. Click **Enqueue Task**, submit a valid task, and confirm it appears in the task list with its activity type, priority, attempt, and status.
6. Open **Dead Letter Queue** and click **Requeue** on the recoverable entry.
7. Confirm the prompt, approve it, refresh both the task and dead-letter views, and verify the entry left the dead-letter queue and returned to task processing.
8. Click the delete action on the disposable entry, reject the first prompt, and confirm the row remains.
9. Repeat the delete action, approve it, and confirm only that entry is removed.
10. If another disposable entry is available, click **Purge All**, reject the first prompt, then approve it and confirm the queue becomes empty.

## Expected Result

- Summary values and task rows reflect backend queue state.
- Status, activity-type, and text filters reduce the task list without changing queue data.
- A valid enqueue creates a task and refresh exposes its lifecycle state.
- Requeue moves the selected dead-letter entry back to task processing.
- Delete and purge require confirmation; canceling leaves data unchanged.
- Approved delete removes one entry, while approved purge removes all remaining entries.
