# TC005: Session List and Filter by Agent

## Description

Verify that sessions can be listed and filtered by agent_id.

## Preconditions

- API server running locally (`just start-dev`) or a deployed API is available
- Set `BASE_URL` to the API origin (for example, `http://localhost:9300`)
- For authenticated deployments, configure `curl` with the required authorization and organization headers
- The organization may contain sessions created by other tests or users

## Test Data

| Field | Value |
|-------|-------|
| Agent A name | session-agent-a |
| Agent B name | session-agent-b |
| Sessions to create | Two for Agent A; one for Agent B |

## Steps

1. Create two agents:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "session-agent-a", "system_prompt": "Agent A."}'

   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "session-agent-b", "system_prompt": "Agent B."}'
   ```
   Save `agent_a_id` and `agent_b_id`.

2. Create sessions for each agent:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_a_id}"}'

   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_a_id}"}'

   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_b_id}"}'
   ```
   Save the returned session IDs as `agent_a_session_1`, `agent_a_session_2`, and `agent_b_session_1`.

3. List all sessions:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions"
   ```

4. Filter sessions by Agent A:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions?agent_id={agent_a_id}"
   ```

5. Filter sessions by Agent B:
   ```bash
   curl -s "${BASE_URL}/api/v1/sessions?agent_id={agent_b_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3: created sessions | All three saved session IDs are present |
| Step 4: created Agent A sessions | Both saved Agent A session IDs are present |
| Step 4: all `agent_id` values | Match `agent_a_id` |
| Step 5: created Agent B session | Saved Agent B session ID is present |
| Step 5: all `agent_id` values | Match `agent_b_id` |
