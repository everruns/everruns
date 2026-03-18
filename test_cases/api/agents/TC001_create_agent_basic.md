# TC001: Create Agent - Basic

## Description

Verify that an agent can be created with required fields (name, system_prompt) and returns a valid agent object.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Name | Test Agent |
| System Prompt | You are a helpful assistant. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Test Agent",
       "system_prompt": "You are a helpful assistant."
     }'
   ```
   Save `id` from response.

2. Fetch agent by ID:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| HTTP status (create) | 201 |
| `id` format | Starts with `agent_` |
| `name` | `"Test Agent"` |
| `system_prompt` | `"You are a helpful assistant."` |
| `status` | `"active"` |
| `capabilities` | `[]` (empty) |
| `tags` | `[]` (empty) |
| GET returns same agent | Fields match create response |

## Validation Commands

```bash
# Assert: agent created with correct name
curl -s -X POST ".../agents" ... | jq '.name == "Test Agent"'

# Assert: agent is active
curl -s ".../agents/{id}" | jq '.status == "active"'

# Assert: ID has correct prefix
curl -s ".../agents/{id}" | jq '.id | startswith("agent_")'
```
