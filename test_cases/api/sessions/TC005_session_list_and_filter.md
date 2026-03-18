# TC005: Session List and Filter by Agent

## Description

Verify that sessions can be listed and filtered by agent_id.

## Preconditions

- API server running (`just start-dev`)

## Steps

1. Create two agents:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "Agent A", "system_prompt": "Agent A."}'

   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "Agent B", "system_prompt": "Agent B."}'
   ```
   Save `agent_a_id` and `agent_b_id`.

2. Create sessions for each agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_a_id}"}'

   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_a_id}"}'

   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_b_id}"}'
   ```

3. List all sessions:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions"
   ```

4. Filter sessions by Agent A:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions?agent_id={agent_a_id}"
   ```

5. Filter sessions by Agent B:
   ```bash
   curl -s "http://localhost:9300/api/v1/sessions?agent_id={agent_b_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3: total sessions | >= 3 |
| Step 4: sessions count | 2 (both for Agent A) |
| Step 4: all `agent_id` values | Match `agent_a_id` |
| Step 5: sessions count | 1 (for Agent B) |
| Step 5: `agent_id` | Matches `agent_b_id` |
