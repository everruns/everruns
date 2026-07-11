# TC008: Session with Model Override

## Description

Verify that a session can override the agent's default model and that the override is used for responses.

## Preconditions

- API server running locally (`just start-dev`) or a deployed API is available
- Set `BASE_URL` to the API origin (for example, `http://localhost:9300`)
- For authenticated deployments, configure `curl` with the required authorization and organization headers
- LLM API keys configured
- At least two models available (check `GET /v1/models`)

## Test Data

| Field | Value |
|-------|-------|
| Agent name | model-test-agent |
| Default model | Any enabled model |
| Override model | A different enabled model |
| User message | Hello. |

## Steps

1. List available models:
   ```bash
   curl -s "${BASE_URL}/api/v1/models"
   ```
   Pick two model IDs.

2. Create agent with default model:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "model-test-agent",
       "system_prompt": "You are a helpful assistant.",
       "default_model_id": "{model_id_1}"
     }'
   ```
   Save `agent_id`.

3. Create session with model override:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{
       "agent_id": "{agent_id}",
       "model_id": "{model_id_2}"
     }'
   ```
   Save `session_id`.

4. Send message:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions/{session_id}/messages" \
     -H "Content-Type: application/json" \
     -d '{
       "message": {
         "role": "user",
         "content": [{"type": "text", "text": "Hello."}]
       }
     }'
   ```

5. Wait for completion, then check events:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions/{session_id}/events"
   ```

6. Fetch the session and confirm its selected model:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Session `model_id` | `{model_id_2}` (override, not agent default) |
| Agent responds | `output.message.completed` event exists |
| Output model metadata | `output.message.completed.message.metadata.model` or `llm.generation.metadata.model` matches `{model_id_2}` |
