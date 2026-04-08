# TC001: Agent Chat - Multi-turn Conversation

## Description

Verify that a user can open a direct chat session with an agent, send a message, wait for the full streamed response, then send a follow-up message in the same session and receive a contextual response.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- LLM API keys configured
- An agent exists with a humor-oriented system prompt (e.g., display name "Dad Jokes", slug `dad-jokes`, system prompt: "You are a dad jokes comedian. Tell funny, clean dad jokes. Always stay in character.")

## Test Data

| Turn | User Message |
|------|-------------|
| 1 | Tell me a dad joke about the current time of day |
| 2 | That was great! Now give me 10 more dad jokes on completely different topics |

## Steps

1. Navigate to the Agents page (`/agents`)
2. Click on the "Dad Jokes" agent card (display name shown prominently, slug `dad-jokes` in monospace underneath) to open its detail page
3. Start a new session / open the chat interface for this agent
4. Send turn 1 message
5. Wait for the full streamed response to complete (spinner/typing indicator disappears, message fully rendered)
6. Verify the response is a dad joke related to time of day
7. Send turn 2 message in the same session
8. Wait for the full streamed response to complete
9. Verify the response contains multiple jokes
10. Verify that the conversation thread shows both user messages and both agent responses in chronological order (turn 1 user + agent, then turn 2 user + agent)
11. Refresh the page (browser reload) or navigate away and return to the same session
12. Verify that the full conversation history (both turns and responses) is still visible and unchanged

## Expected Result

| Check | Expected |
|-------|----------|
| Session created | A new session is created for the Dad Jokes agent |
| Turn 1 response streams | Response streams in progressively, typing indicator visible during generation |
| Turn 1 response complete | Full response rendered, no truncation, typing indicator gone |
| Turn 1 content | Response contains a dad joke referencing time of day |
| Turn 2 sends in same session | Follow-up message appears in the same conversation thread below turn 1 |
| Turn 2 response streams | Second response streams in progressively |
| Turn 2 response complete | Full response rendered with multiple jokes |
| Turn 2 content | Response contains approximately 10 jokes on varied topics |
| Conversation history | Both user messages and both agent responses visible in the chat, in correct order |
| Session persists | Refreshing the page and reopening the session shows the full conversation history |
