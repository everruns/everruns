# TC001: Platform Chat Create and Run Simple Agent

## Description

Verify that a user can use **Platform Chat** to create a simple agent and then run that same agent from the same chat session. Also verify that clear errors appear when the user requests an invalid agent creation or tries to run an agent that does not exist.

## Preconditions

- User is signed in
- The sidebar **Chat** entry is available
- No existing agent uses the slug `weather-bot`

## Test Data

| Field | Value |
|-------|-------|
| Create message | Create a demo agent named "weather-bot" that answers weather questions. |
| Run message | Now run weather-bot and ask it: what should I wear if it's 5°C with light rain? |
| Invalid create message | Create an agent with an empty name that answers weather questions. |
| Missing agent message | Run agent does-not-exist and ask it: what should I wear if it's 5°C with light rain? |

## Steps

### Happy Path

1. Sign in and open **Chat** from the sidebar.
2. In a new Platform Chat conversation, send the create message from **Test Data**.
3. Wait for Platform Chat to confirm the agent was created.
4. Open the link from the confirmation message and verify the agent page loads for `weather-bot`.
5. Open the **Agents** page and verify `weather-bot` appears in the list.
6. Return to the same Platform Chat conversation.
7. Send the run message from **Test Data**.
8. Wait for Platform Chat to return the agent's reply.

### Negative Paths

9. In a new Platform Chat conversation, send the invalid create message from **Test Data**.
10. Observe the error shown in the chat.
11. In another new Platform Chat conversation, send the missing agent message from **Test Data**.
12. Observe the error shown in the chat.

## Expected Result

### Happy Path

| Check | Expected |
|-------|----------|
| Create acknowledgement | Platform Chat confirms the agent was created |
| Agent link | The confirmation includes a clickable link to the created agent |
| Agent page | The linked agent page loads successfully for `weather-bot` |
| Agents list | `weather-bot` appears on the **Agents** page immediately after creation |
| Run acknowledgement | Platform Chat acknowledges the request to run `weather-bot` |
| Agent reply | The answer is coherent and weather-appropriate, mentioning layered clothing and rain protection |
| Transcript rendering | Tool activity appears as structured tool blocks, not raw `to=functions...` text |
| Error handling | No error banner appears during the happy path |
| Fallback text | The transcript does not show `I encountered an error while processing your request` or `Execution stopped because the assigned harness was deleted` |

### Negative Paths

| Check | Expected |
|-------|----------|
| Empty name validation | Platform Chat shows a clear error instead of creating an agent |
| Missing agent validation | Platform Chat shows a clear agent-not-found style message |
| Negative-path stability | Neither negative-path conversation silently fails or hangs without a response |

## Notes

- Keep both happy-path actions in the same Platform Chat conversation.
- Exact wording may vary by model; judge the outcome by whether the response is clear, on-topic, and actionable.
