# TC008: Global Chat - Multi-turn Agent Management

## Description

Verify that global chat supports multi-turn conversation for agent management: create an agent, run it, check results, then update the agent — all in one conversation.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled
- LLM API keys configured

## Test Data

| Turn | User Message |
|------|-------------|
| 1 | Create an agent called "Joke Bot" with system prompt "You tell short, clean jokes." |
| 2 | Run Joke Bot with the task: "Tell me a joke about programming." |
| 3 | What did Joke Bot say? |
| 4 | List all my agents. |

## Steps

1. Navigate to `/chat`
2. Send turn 1 message, confirm creation if asked
3. Wait for response, verify agent created
4. Send turn 2 message
5. Wait for response, verify Joke Bot ran and returned a joke
6. Send turn 3 message
7. Verify chat agent recalls the Joke Bot's response from context
8. Send turn 4 message
9. Verify chat agent lists all agents including Joke Bot

## Expected Result

| Check | Expected |
|-------|----------|
| Turn 1 | Agent "Joke Bot" created with link |
| Turn 2 | Session created, joke returned |
| Turn 3 | Chat agent recalls the joke from previous turn |
| Turn 4 | Agent list returned, includes Joke Bot |
| Conversation continuity | All turns share the same global chat session |
