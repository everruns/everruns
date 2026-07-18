# TC007 Scoped agent and user memory

## Description

Verify that host-agent memory is mounted at `/memory/agent` and persists across two sessions of the same agent, while user memory is mounted only in sessions where that user participates.

## Preconditions

- A local or authenticated Everruns stack with scoped memory enabled is available
- The tester is signed in as User A
- A second account, User B, can access the same organization
- An active agent with workspace file tools is available

## Test Data

| Field | Value |
| --- | --- |
| Agent memory file | `/memory/agent/tc007-agent-note.txt` |
| Agent memory content | `agent memory persists across sessions` |
| User memory file | `/memory/user/tc007-user-note.txt` |
| User memory content | `private to participating user` |

## Steps

1. As User A, create Session 1 hosted by the test agent.
2. Ask the agent to list `/memory` and verify both `/memory/agent` and `/memory/user` are mounted.
3. Ask the agent to write the agent and user test files with the specified contents.
4. End Session 1 and create Session 2 with the same host agent as User A.
5. Read `/memory/agent/tc007-agent-note.txt` and verify the content written in Session 1 persists.
6. Read `/memory/user/tc007-user-note.txt` and verify User A's content persists in Session 2.
7. As User B, create Session 3 with the same host agent without adding User A as a participant.
8. Verify `/memory/agent/tc007-agent-note.txt` is present because the host agent is the same.
9. Verify User A's `/memory/user/tc007-user-note.txt` is not mounted or readable in Session 3.
10. Ask the agent to write `/memory/user/tc007-user-b-note.txt` in Session 3, then verify that file is visible to User B but not in either of User A's sessions.

## Expected Result

- The host agent's memory is automatically mounted read-write at `/memory/agent`.
- Agent memory written in one session persists into a second session hosted by the same agent.
- User A's memory is mounted at `/memory/user` in User A's sessions and persists for User A.
- A session where User A does not participate cannot read or expose User A's scoped memory.
- Each user's memory is mounted only in sessions where that user participates; another user's private memory is never merged into it.
