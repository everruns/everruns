# TC004: Update Agent

## Description

Verify that an agent can be partially updated via PATCH, including name (slug) and display_name independently, and that only specified fields change.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Original Name | original-agent |
| Original Display Name | Original Agent |
| Updated Name | updated-agent |
| Updated Display Name | Updated Agent |
| Original Prompt | You are original. |
| Updated Prompt | You are updated. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "original-agent",
       "display_name": "Original Agent",
       "system_prompt": "You are original.",
       "tags": ["v1"]
     }'
   ```
   Save `id`.

2. Patch agent (update name only):
   ```bash
   curl -s -X PATCH "http://localhost:9300/api/v1/agents/{id}" \
     -H "Content-Type: application/json" \
     -d '{"name": "updated-agent"}'
   ```

3. Fetch agent:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

4. Patch agent (update display_name only):
   ```bash
   curl -s -X PATCH "http://localhost:9300/api/v1/agents/{id}" \
     -H "Content-Type: application/json" \
     -d '{"display_name": "Updated Agent"}'
   ```

5. Patch agent (update prompt and tags):
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
| Step 3: `name` | `"updated-agent"` |
| Step 3: `display_name` | `"Original Agent"` (unchanged) |
| Step 3: `system_prompt` | `"You are original."` (unchanged) |
| Step 3: `tags` | `["v1"]` (unchanged) |
| Step 4: `display_name` | `"Updated Agent"` |
| Step 4: `name` | `"updated-agent"` (unchanged) |
| Step 5: `system_prompt` | `"You are updated."` |
| Step 5: `tags` | `["v2"]` |
| `updated_at` | Changes after each PATCH |
