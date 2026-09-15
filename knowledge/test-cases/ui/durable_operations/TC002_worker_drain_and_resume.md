---
type: Test Case
title: "TC002: Worker drain and resume"
description: "Verify that the durable workers UI reports capacity and heartbeat state and lets an operator drain and resume a worker."
tags:
  - everruns
  - test-case
  - ui
  - durable
---
# TC002: Worker drain and resume

## Description

Verify that the durable workers UI reports capacity and heartbeat state and lets an operator drain and resume a worker.

## Preconditions

- The full stack is running and the user is signed in as an operator.
- At least one active worker has a nonzero concurrency limit and registered activity types.
- A task can be started long enough to make worker load visible.

## Test Data

| Field | Value |
|---|---|
| Route | `/durable/workers` |
| Worker | An active test worker |
| Expected group | The worker's configured group |

## Steps

1. Open `/durable/workers`.
2. Confirm the summary shows total workers, total capacity, current load, and draining/stopped counts.
3. Locate the test worker and verify its status, group, load fraction, accepting state, activity types, completed/failed counts, average duration, and last heartbeat.
4. Start a task and refresh until the worker load increases, then let the task finish and verify the completed count or load changes.
5. Click **Drain** for the active worker and reject the confirmation prompt.
6. Confirm the worker remains active and accepting tasks.
7. Click **Drain** again, approve the prompt, and refresh until the row reports `draining` and **Accepting** is `No`.
8. Click **Resume** and refresh until the row reports `active` and **Accepting** is `Yes`.

## Expected Result

- Summary capacity and load agree with the worker rows.
- Worker state and heartbeat information update after refresh.
- Canceling the drain prompt makes no change.
- Approved drain stops new task acceptance without removing the worker.
- Resume returns the worker to active task acceptance.
