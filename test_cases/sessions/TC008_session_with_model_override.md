# TC008: Session with Model Override

## Description

Verify that a session can override the agent's default model and that the override is used for responses.

## Preconditions

- API server running (`just start-dev`)
- LLM API keys configured
- At least two models available (check `GET /v1/models`)

## Steps

1. List available models:
   ```bash
   curl -s "http://localhost:9300/api/v1/models"
   ```
   Pick two model IDs.

2. Create agent with default model:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Model Test Agent",
       "system_prompt": "You are a helpful assistant.",
       "default_model_id": "{model_id_1}"
     }'
   ```
   Save `agent_id`.

3. Create session with model override:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{
       "agent_id": "{agent_id}",
       "model_id": "{model_id_2}"
     }'
   ```
   Save `session_id`.

4. Send message:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
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
   curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Session `model_id` | `{model_id_2}` (override, not agent default) |
| `reason.completed` event | `model` field matches `{model_id_2}` |
| Agent responds | `output.message.completed` event exists |
