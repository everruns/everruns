# TC007 Scoped agent and user memory

## Description

Verify that agent-scoped and user-scoped memory automatically mount read-write at `/memory/agent` and `/memory/user`, persist across sessions of the same host agent and the same user, and that the `/memory/*` namespace is reserved. Distinct from the organization-memory pages covered by TC001–TC006.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An active agent is available to host sessions
- The tester can open the Workspace / file view of a session

## Test Data

| Field | Value |
| --- | --- |
| Host agent | Notetaker |
| Agent memory file | `/memory/agent/known-issues.md` |
| User memory file | `/memory/user/preferences.md` |
| Content | `Prefer concise summaries` |

## Steps

1. Start a session hosted by the agent and open the Workspace / file view.
2. Confirm `/memory/agent` is present and mounted read-write (its files show an agent-memory indicator).
3. Create or edit a file under `/memory/agent` (for example `known-issues.md`), enter the content, and save.
4. In the owner's private default session, confirm `/memory/user` is present and mounted read-write, then write a file under it (for example `preferences.md`) and save.
5. Start a second, separate session hosted by the **same** agent.
6. Verify the `/memory/agent` file written in step 3 is present with its content.
7. In the owner's later private session, verify the `/memory/user` file written in step 4 is present with its content.
8. Attempt to add a caller-supplied initial file, or configure a `memory` capability mount, under `/memory/` and confirm it is rejected.

## Expected Result

- `/memory/agent` and `/memory/user` mount automatically and read-write, with no manual capability configuration.
- Files written to `/memory/agent` reappear in later sessions hosted by the same agent (agent memory follows the host agent).
- Files written to `/memory/user` reappear in the same owner's later sessions (user memory follows the user).
- The `/memory/*` namespace is reserved: caller-supplied initial files and public `memory` capability mounts under it are rejected.
- User memory is private to the owner: `/memory/user` is not readable by other users and is redacted from their file searches. (Verify with a second user when available.)
