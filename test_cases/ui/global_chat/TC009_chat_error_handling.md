# TC009: Global Chat - Error Handling for Agent Operations

## Description

Verify that global chat handles error cases gracefully: running a nonexistent agent, creating a duplicate agent name, and interacting with invalid references.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled
- LLM API keys configured

## Test Data

| Turn | User Message |
|------|-------------|
| 1 | Run an agent called "Nonexistent Bot 999" with task "hello" |
| 2 | Create an agent with an empty name |
| 3 | List all agents and tell me how many there are |

## Steps

1. Navigate to `/chat`
2. Send turn 1 — attempt to run a nonexistent agent
3. Observe the response: should indicate the agent was not found
4. Send turn 2 — attempt invalid creation
5. Observe the response: should indicate validation error or refuse
6. Send turn 3 — verify the chat agent can still function after errors

## Expected Result

| Check | Expected |
|-------|----------|
| Nonexistent agent | Chat agent reports agent not found or offers to create it |
| Empty name | Chat agent refuses or reports validation error |
| Recovery | Turn 3 succeeds — chat agent lists agents correctly |
| No crash | Chat interface remains functional throughout |
| No orphan sessions | No broken sessions created for nonexistent agent |
