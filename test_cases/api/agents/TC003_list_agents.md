# TC003: List Agents

## Description

Verify that agents are listed correctly, with pagination, and that archived agents are excluded by default. Verify both name (slug) and display_name appear in list responses.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Agent 1 Name | agent-alpha |
| Agent 1 Display Name | Agent Alpha |
| Agent 2 Name | agent-beta |
| Agent 2 Display Name | Agent Beta |

## Steps

1. Create two agents:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "agent-alpha", "display_name": "Agent Alpha", "system_prompt": "Alpha agent."}'

   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "agent-beta", "display_name": "Agent Beta", "system_prompt": "Beta agent."}'
   ```
   Save both `id` values.

2. List agents:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents"
   ```

3. Archive one agent:
   ```bash
   curl -s -X DELETE "http://localhost:9300/api/v1/agents/{agent_alpha_id}"
   ```

4. List agents again (default, no archived):
   ```bash
   curl -s "http://localhost:9300/api/v1/agents"
   ```

5. List agents including archived:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents?include_archived=true"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 2: both agents in list | `data` contains both agent IDs |
| Step 2: name fields | Each agent has `name` (slug) and `display_name` |
| Step 3: archive returns 200 | Agent status becomes `"archived"` |
| Step 4: archived agent excluded | `data` does not contain agent-alpha |
| Step 4: active agent included | `data` contains agent-beta |
| Step 5: both agents in list | `data` contains both agents |
| Step 5: archived agent status | agent-alpha has `status: "archived"` |
