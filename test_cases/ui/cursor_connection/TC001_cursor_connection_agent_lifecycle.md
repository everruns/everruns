# TC001: Cursor Connection - Cloud Agent Lifecycle

## Description

Verify that the Cursor Agent Manager prompts for a Cursor API key via the inline connection dialog, validates the key, and can launch then inspect a Cursor Cloud Agent.

## Preconditions

- Server running (`just start-all` recommended)
- User logged in
- LLM API key configured
- No existing Cursor connection in Settings > Connections
- Valid Cursor Cloud Agents API key available
- Cursor GitHub app has access to the test repository
- Use a small test repository where creating a branch/PR is safe

## Test Data

| Field | Value |
|-------|-------|
| Agent | Cursor Agent Manager |
| Repository | `https://github.com/<org>/<safe-test-repo>` |
| Base Ref | `main` |
| First Message | Launch a Cursor agent for `<repo>` on `main` that only updates a scratch README note. Use branch `cursor/everruns-smoke-test`, do not auto-create a PR, then report the agent id and status. |
| Cursor API Key | Valid Cursor Cloud Agents API key |
| Cleanup Message | Check the Cursor agent status, read the conversation, then delete the Cursor agent record. |

## Steps

1. Navigate to the Agents page and locate **Cursor Agent Manager**
2. Click **Run** to start a new session
3. Send the first message from Test Data
4. Wait for the inline **Setup Connection** card for Cursor
5. Click **Connect**
6. Verify the API key dialog mentions Cursor Cloud Agents
7. Enter the Cursor API key and submit
8. Wait for the agent to resume and call `cursor_launch_agent`
9. Verify the response includes a Cursor `agent_id`, Cursor web URL, branch name, and initial status
10. Send the cleanup message
11. Verify the agent calls `cursor_get_agent`
12. Verify the agent calls `cursor_get_conversation`
13. Verify the agent calls `cursor_delete_agent`

## Expected Result

| Check | Expected |
|-------|----------|
| Connection prompt | Inline Cursor setup card appears |
| Dialog copy | Shows Cursor provider name and Cloud Agents API key guidance |
| Connection validation | Dialog closes after valid key submission |
| Agent launched | `cursor_launch_agent` returns a Cursor `agent_id` and web URL |
| Status check | `cursor_get_agent` returns status without error |
| Conversation check | `cursor_get_conversation` returns messages |
| Cleanup | `cursor_delete_agent` returns the deleted agent id |

## Cleanup

- Delete any branch or PR created in the safe test repository
- Remove the Cursor connection from Settings > Connections if it should not persist
