# TC005: Delete Agent - Archive and Hard Delete

## Description

Verify the two-stage agent deletion: soft delete (archive) via DELETE, then hard delete via POST /delete.

## Preconditions

- API server running (`just start-dev`)

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "doomed-agent", "display_name": "Doomed Agent", "system_prompt": "Temporary."}'
   ```
   Save `id`.

2. Archive agent (soft delete):
   ```bash
   curl -s -X DELETE "http://localhost:9300/api/v1/agents/{id}"
   ```

3. Verify archived agent is still accessible:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

4. Hard delete:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents/{id}/delete"
   ```

5. Verify agent is gone:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" "http://localhost:9300/api/v1/agents/{id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 2: HTTP status | 200 |
| Step 2: `status` | `"archived"` |
| Step 3: agent accessible | Returns agent with `status: "archived"` |
| Step 4: HTTP status | 200 |
| Step 5: HTTP status | 404 |
