# TC007: Delete Session

## Description

Verify that a session can be deleted and is no longer accessible.

## Preconditions

- API server running locally (`just start-dev`) or a deployed API is available
- Set `BASE_URL` to the API origin (for example, `http://localhost:9300`)
- For authenticated deployments, configure `curl` with the required authorization and organization headers

## Test Data

| Field | Value |
|-------|-------|
| Agent name | temp-agent |
| Agent prompt | Temporary. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "temp-agent", "system_prompt": "Temporary."}'
   ```
   Save `agent_id`.

2. Create session:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id`.

3. Delete session:
   ```bash
   curl -s -X DELETE "${BASE_URL}/api/v1/sessions/{session_id}"
   ```

4. Attempt to fetch deleted session:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" "${BASE_URL}/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3: HTTP status | 204 with no response body |
| Step 4: HTTP status | 404 |
