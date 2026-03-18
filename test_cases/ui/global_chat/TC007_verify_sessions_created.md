# TC007: Global Chat - Verify Sessions Created by Agent Runs

## Description

After running agents via global chat, verify that sessions were created and are visible in the Sessions page with correct agent associations and idle status.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- 10 agents run via global chat (TC005 completed)

## Test Data

None (uses sessions created by TC005).

## Steps

1. Navigate to `/sessions`
2. Observe the sessions list
3. For each of the 10 agent-linked sessions:
   - Verify agent name is displayed
   - Click into the session
   - Check session status and messages

## Expected Result

| Check | Expected |
|-------|----------|
| 10 agent sessions | At least 10 sessions with agent assignments visible |
| Agent names | Each session shows the correct agent name |
| Status | All sessions are `idle` (turn completed) |
| Messages | Each session has >= 2 messages (user + agent response) |
| Chat history | Clicking a session shows the conversation with task and response |
