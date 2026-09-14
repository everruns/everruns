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
- The queue contains pending, claimed, completed, and failed tasks, including failed tasks from at least two activity types.
- The dead-letter queue contains one recoverable entry, one entry for single-row deletion, and one entry reserved for purge.
- Approve delete or purge actions only on an isolated local or test stack whose dead-letter queue contains only entries created for this case; on shared stacks, reject the delete and purge confirmation prompts.

## Test Data

| Field | Value |
|---|---|
| Route | `/durable/queues` |
| Search value | A known activity ID |
| Status filter | `failed` |
| Activity type filter | An activity type used by a known failed task |
| Recoverable entry | A task that succeeds when retried |

## Steps

1. Open `/durable/queues` and compare the Pending, Processing, Completed/hr, and Failed/hr totals with the seeded task state.
2. Confirm the oldest-pending, average-wait, and average-execution values are present.
3. Open **Tasks**, select the `failed` status, and confirm only failed tasks remain.
4. Select the known activity type and confirm every remaining task has that type, then clear the activity-type filter.
5. Search for the known activity ID, then clear the search and status filters.
6. Click **Enqueue Task**, submit a valid task, and confirm it appears in the task list with its activity type, priority, attempt, and status.
7. Open **Dead Letter Queue** and click **Requeue** on the recoverable entry.
8. Confirm the prompt, approve it, refresh both the task and dead-letter views, and verify the entry left the dead-letter queue and returned to task processing.
9. Click the delete action on the single-row deletion entry, reject the prompt, and confirm the row remains.
10. On an isolated stack only, repeat the delete action, approve it, and confirm only that entry is removed.
11. Click **Purge All**, reject the prompt, and confirm all rows remain.
12. On an isolated stack only, ensure every remaining row belongs to this case, repeat **Purge All**, approve it, and confirm the queue becomes empty.

## Expected Result

- Summary values and task rows reflect backend queue state.
- Status, activity-type, and text filters reduce the task list without changing queue data.
- A valid enqueue creates a task and refresh exposes its lifecycle state.
- Requeue moves the selected dead-letter entry back to task processing.
- Delete and purge require confirmation; canceling leaves data unchanged.
- On an isolated stack, approved delete removes one entry and approved purge removes only this case's remaining entries.
- On a shared stack, the case never approves delete or purge.
