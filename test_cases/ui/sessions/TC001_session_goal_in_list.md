## Description

Verifies that a session with a goal displays that goal in the sessions list.

## Preconditions

- The API/UI stack is running.
- The signed-in user can create sessions.
- A session exists with title `Detached research peer` and goal `Investigate queue latency regression`.

## Test Data

| Field | Value |
| --- | --- |
| Session title | `Detached research peer` |
| Session goal | `Investigate queue latency regression` |

## Steps

1. Navigate to `/sessions`.
2. Locate the `Detached research peer` session card.
3. Inspect the text shown under the session title.

## Expected Result

- The session card is visible in the sessions list.
- The card displays `Investigate queue latency regression`.
- Existing preview/output preview text remains visible below the goal when present.
