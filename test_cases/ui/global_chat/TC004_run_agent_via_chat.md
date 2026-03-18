# TC004: Global Chat - Run Agent via Chat

## Description

Verify that the global chat agent can run a previously created agent by creating a session, sending a message, waiting for completion, and relaying results.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `global_chat` enabled
- LLM API keys configured
- At least one agent exists (e.g., "Math Tutor" from TC003)

## Test Data

| Field | Value |
|-------|-------|
| User Message | Run the Math Tutor agent with the task: "What is the square root of 144?" |

## Steps

1. Navigate to `/chat`
2. Send the message from test data above
3. Wait for the chat agent to:
   - Create a session for Math Tutor
   - Send the task message
   - Wait for the turn to complete
   - Retrieve and relay the results
4. Observe the response

## Expected Result

| Check | Expected |
|-------|----------|
| Session created | Chat agent creates session for Math Tutor |
| Message sent | Task forwarded to the session |
| Result relayed | Response includes the Math Tutor's answer (mentions "12") |
| Session link | Response contains link to the session |
| Session visible | New session appears at `/sessions` with Math Tutor agent |
