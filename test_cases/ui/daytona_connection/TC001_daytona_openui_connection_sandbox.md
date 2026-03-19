# TC001: Daytona OpenUI Connection - Sandbox Lifecycle

## Description

Verify that the Daytona Coder agent prompts for a Daytona API key via the inline OpenUI connection dialog when no connection exists, creates a sandbox after connecting, executes a command, and deletes the sandbox on request.

## Preconditions

- Server running (`just start-all` — full mode with PostgreSQL required for leased resources)
- User logged in
- LLM API keys configured (Anthropic or OpenAI)
- **No** existing Daytona connection in Settings > Connections (disconnect first if present)
- Valid Daytona API key available for test

## Test Data

| Field | Value |
|-------|-------|
| Agent | Daytona Coder (seed agent) |
| First Message | Create a sandbox and calculate the result of 123 * 456. Do NOT delete the sandbox after - I want to keep it running. |
| Daytona API Key | *(use a valid Daytona API key)* |
| Cleanup Message | Delete the sandbox |

## Steps

1. Navigate to the Agents page and locate **Daytona Coder**
2. Click **Run** to start a new session
3. Send the message: `Create a sandbox and calculate the result of 123 * 456. Do NOT delete the sandbox after - I want to keep it running.`
4. Wait for the inline **Setup Connection** card to appear in the chat (OpenUI connection prompt for Daytona)
5. Click the **Connect** button on the inline card
6. In the API Key dialog:
   - Verify the dialog shows Daytona provider name and instructions
   - Enter the Daytona API key
   - Click **Submit**
7. Wait for the connection to be validated and the dialog to close
8. Wait for the agent to resume and create a sandbox (tool call: `daytona_create_sandbox`)
9. Wait for the agent to execute the calculation (tool call: `daytona_exec`)
10. Verify the agent responds with the correct result (123 * 456 = 56088)
11. Send the message: `Delete the sandbox`
12. Wait for the agent to delete the sandbox (tool call: `daytona_manage_sandbox` with action "delete")
13. Verify the agent confirms the sandbox has been **deleted**

## Notes

- The Daytona Coder agent's system prompt says "Always delete sandboxes when done." To test explicit deletion as a separate step, the first message must instruct the agent **not** to delete the sandbox.
- **DEV_MODE limitation**: The connection resume flow (steps 7–8) may fail in DEV_MODE due to a race condition in the in-memory message store. Use `just start-all` (PostgreSQL) for reliable testing of the full connection flow.

## Expected Result

| Check | Expected |
|-------|----------|
| Connection prompt | Inline "Setup Connection" card appears for Daytona provider |
| API Key dialog | Shows Daytona icon, provider name, and instructions |
| Connection saved | After submit, dialog closes and connection is stored |
| Sandbox created | Agent creates sandbox successfully (sandbox_id returned) |
| Command executed | Agent runs calculation, result includes 56088 |
| Sandbox deleted | Agent confirms sandbox is deleted |
| Resources cleaned | Session resources page (`/sessions/{id}/resources`) shows the sandbox as "Released" |

## Cleanup

- After the test, navigate to **Settings > Connections** and verify or disconnect the Daytona connection if it should not persist
- If the sandbox deletion step failed, manually delete any remaining sandboxes via the Daytona dashboard to avoid resource leaks
