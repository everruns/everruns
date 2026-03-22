# TC001: Deno OpenUI Connection - Sandbox Lifecycle

## Description

Verify that an agent with Deno capability prompts for a Deno access token via the inline OpenUI connection dialog when no connection exists, creates a sandbox after connecting, executes a command, and deletes the sandbox on request.

## Preconditions

- Server running (`just start-all` — full mode with PostgreSQL required for leased resources)
- User logged in
- LLM API keys configured (Anthropic or OpenAI)
- **No** existing Deno connection in Settings > Connections (disconnect first if present)
- Valid Deno organization access token (`ddo_...`) available for test

## Test Data

| Field | Value |
|-------|-------|
| Agent | Deno Coder (seed agent) |
| First Message | Create a sandbox and calculate the result of 123 * 456. Do NOT delete the sandbox after - I want to keep it running. |
| Deno Access Token | *(use a valid Deno organization token, `ddo_...`)* |
| Cleanup Message | Delete the sandbox |

## Steps

1. Navigate to the Agents page and locate **Deno Coder**
2. Click **Run** to start a new session
3. Send the message: `Create a sandbox and calculate the result of 123 * 456. Do NOT delete the sandbox after - I want to keep it running.`
4. Wait for the inline **Setup Connection** card to appear in the chat (OpenUI connection prompt for Deno)
5. Click the **Connect** button on the inline card
6. In the Access Token dialog:
   - Verify the dialog shows Deno Deploy provider name and instructions
   - Enter the Deno organization access token
   - Click **Submit**
7. Wait for the connection to be validated and the dialog to close
8. Wait for the agent to resume and create a sandbox (tool call: `deno_create_sandbox`)
9. Wait for the agent to execute the calculation (tool call: `deno_exec`)
10. Verify the agent responds with the correct result (123 * 456 = 56088)
11. Send the message: `Delete the sandbox`
12. Wait for the agent to delete the sandbox (tool call: `deno_manage_sandbox` with action "delete")
13. Verify the agent confirms the sandbox has been **deleted**

## Notes

- The Deno Coder agent's system prompt says "Always delete sandboxes when done." To test explicit deletion as a separate step, the first message must instruct the agent **not** to delete the sandbox.
- Deno sandboxes use websocket connections, so sandbox creation may take a few seconds longer than REST-based integrations.
- **DEV_MODE limitation**: The connection resume flow (steps 7–8) may fail in DEV_MODE due to a race condition in the in-memory message store. Use `just start-all` (PostgreSQL) for reliable testing of the full connection flow.

## Expected Result

| Check | Expected |
|-------|----------|
| Connection prompt | Inline "Setup Connection" card appears for Deno provider |
| Access Token dialog | Shows Deno Deploy icon, provider name, and instructions |
| Connection saved | After submit, dialog closes and connection is stored |
| Sandbox created | Agent creates sandbox successfully (sandbox_id returned) |
| Command executed | Agent runs calculation, result includes 56088 |
| Sandbox deleted | Agent confirms sandbox is deleted |
| Resources cleaned | Session resources page (`/sessions/{id}/resources`) shows the sandbox as "Released" |

## Cleanup

- After the test, navigate to **Settings > Connections** and verify or disconnect the Deno connection if it should not persist
- If the sandbox deletion step failed, manually delete any remaining sandboxes via the Deno Deploy Console to avoid resource leaks
