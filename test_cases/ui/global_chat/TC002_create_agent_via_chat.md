# TC002: Global Chat - Create Agent via Chat

## Description

Verify that the global chat agent can create a new agent when asked, using the `manage_agents` platform management tool.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled
- LLM API keys configured

## Test Data

| Field | Value |
|-------|-------|
| User Message | Create an agent called "Weather Bot" with system prompt "You answer weather questions." |

## Steps

1. Navigate to `/chat`
2. Type: `Create an agent called "Weather Bot" with system prompt "You answer weather questions."`
3. Send the message
4. Wait for the agent to respond (may ask for confirmation, confirm if so)
5. Observe the response — should contain a clickable link to the new agent

## Expected Result

| Check | Expected |
|-------|----------|
| Agent created | Response mentions agent creation success |
| Agent link | Response contains markdown link `[Weather Bot](/agents/agent_...)` |
| Navigate to link | Agent detail page loads with name "Weather Bot" |
| Agent prompt | System prompt is "You answer weather questions." |
| Agent status | `active` |
| Agents list | New agent visible at `/agents` |
