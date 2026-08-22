# TC014: Chats - Harness-Bound Thread

## Description

Verify that a user can start a chat directly from an active Harness without creating or selecting
an Agent, and that the resulting thread remains bound to that Harness.

## Preconditions

- DB-backed stack running
- User can access Chats
- The active built-in Generic Harness is available

## Test Data

| Field | Value |
|-------|-------|
| Harness | Generic |
| Message | `Reply with exactly: harness chat works` |

## Steps

1. Navigate to `/chats/new`.
2. Open the counterpart picker and verify it separates Harnesses from Agents.
3. Select **Generic** under Harnesses and press **Start chat**.
4. Inspect the `POST /v1/sessions` request and response.
5. Verify the browser lands on `/chats/{sessionId}` and the thread header names **Generic**.
6. Send the test message and wait for the response.
7. Return to `/chats` and verify the new thread is listed with its Harness-derived avatar.

## Expected Result

| Check | Expected |
|-------|----------|
| Picker | Active Harnesses and Agents appear in separate labelled groups |
| Request | Session creation sends `harness_name: "generic"` and no Agent binding |
| Response | The server returns 201 with the Generic Harness ID and `agent_id: null` |
| Thread | `/chats/{sessionId}` loads with **Generic** as the fixed counterpart |
| Conversation | The message sends successfully and the Harness produces `harness chat works` |
| Thread list | The new thread appears with its Harness-derived avatar |
