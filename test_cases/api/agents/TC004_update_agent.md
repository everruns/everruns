# TC004: Update Agent

## Description

Verify that an agent can be partially updated via PATCH and that only specified fields change.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Original Name | Original Agent |
| Updated Name | Updated Agent |
| Original Prompt | You are original. |
| Updated Prompt | You are updated. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Original Agent",
       "system_prompt": "You are original.",
       "tags": ["v1"]
     }'
   ```
   Save `id`.

2. Patch agent (update name only):
   ```bash
   curl -s -X PATCH "http://localhost:9300/api/v1/agents/{id}" \
     -H "Content-Type: application/json" \
     -d '{"name": "Updated Agent"}'
   ```

3. Fetch agent:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

4. Patch agent (update prompt and tags):
   ```bash
   curl -s -X PATCH "http://localhost:9300/api/v1/agents/{id}" \
     -H "Content-Type: application/json" \
     -d '{
       "system_prompt": "You are updated.",
       "tags": ["v2"]
     }'
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 2: HTTP status | 200 |
| Step 3: `name` | `"Updated Agent"` |
| Step 3: `system_prompt` | `"You are original."` (unchanged) |
| Step 3: `tags` | `["v1"]` (unchanged) |
| Step 4: `system_prompt` | `"You are updated."` |
| Step 4: `tags` | `["v2"]` |
| `updated_at` | Changes after each PATCH |
