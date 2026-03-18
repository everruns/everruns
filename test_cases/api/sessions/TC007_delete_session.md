# TC007: Delete Session

## Description

Verify that a session can be deleted and is no longer accessible.

## Preconditions

- API server running (`just start-dev`)

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "Temp Agent", "system_prompt": "Temporary."}'
   ```
   Save `agent_id`.

2. Create session:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id`.

3. Delete session:
   ```bash
   curl -s -X DELETE "http://localhost:9300/api/v1/sessions/{session_id}"
   ```

4. Attempt to fetch deleted session:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" "http://localhost:9300/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3: HTTP status | 200 |
| Step 4: HTTP status | 404 |
