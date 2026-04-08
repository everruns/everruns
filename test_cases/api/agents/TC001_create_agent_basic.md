# TC001: Create Agent - Basic

## Description

Verify that an agent can be created with required fields (name, system_prompt) plus optional display_name, and returns a valid agent object with both name and display_name.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Name (slug) | test-agent |
| Display Name | Test Agent |
| System Prompt | You are a helpful assistant. |

## Steps

1. Create agent with both name and display_name:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "test-agent",
       "display_name": "Test Agent",
       "system_prompt": "You are a helpful assistant."
     }'
   ```
   Save `id` from response.

2. Fetch agent by ID:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

3. Fetch agent by name:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/test-agent"
   ```

4. Create agent without display_name (should fall back to name):
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "no-display-name",
       "system_prompt": "Fallback test."
     }'
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| HTTP status (create) | 201 |
| `id` format | Starts with `agent_` |
| `name` | `"test-agent"` |
| `display_name` | `"Test Agent"` |
| `system_prompt` | `"You are a helpful assistant."` |
| `status` | `"active"` |
| `capabilities` | `[]` (empty) |
| `tags` | `[]` (empty) |
| GET by ID returns same agent | Fields match create response |
| GET by name returns same agent | Fields match create response |
| Step 4: `display_name` | `null` (not set, UI falls back to name) |

## Validation Commands

```bash
# Assert: agent created with correct addressable name
curl -s -X POST ".../agents" ... | jq '.name == "test-agent"'

# Assert: display_name is set
curl -s ".../agents/{id}" | jq '.display_name == "Test Agent"'

# Assert: agent is active
curl -s ".../agents/{id}" | jq '.status == "active"'

# Assert: agent accessible by name
curl -s ".../agents/test-agent" | jq '.name == "test-agent"'
```
