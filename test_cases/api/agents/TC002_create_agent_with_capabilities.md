# TC002: Create Agent - With Capabilities

## Description

Verify that an agent can be created with capabilities and that the capabilities are correctly stored and returned.

## Preconditions

- API server running (`just start-dev`)

## Test Data

| Field | Value |
|-------|-------|
| Name | capable-agent |
| Display Name | Capable Agent |
| System Prompt | You are an assistant with tools. |
| Capabilities | `current_time`, `web_fetch` |
| Tags | `["test", "capable"]` |

## Steps

1. Create agent with capabilities:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "capable-agent",
       "display_name": "Capable Agent",
       "system_prompt": "You are an assistant with tools.",
       "capabilities": [
         {"ref": "current_time"},
         {"ref": "web_fetch", "config": {"timeout_ms": 30000}}
       ],
       "tags": ["test", "capable"]
     }'
   ```
   Save `id` from response.

2. Fetch agent:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/{id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| HTTP status | 201 |
| `capabilities` length | 2 |
| `capabilities[0].ref` | `"current_time"` |
| `capabilities[1].ref` | `"web_fetch"` |
| `capabilities[1].config.timeout_ms` | `30000` |
| `tags` | `["test", "capable"]` |

## Validation Commands

```bash
# Assert: capabilities stored correctly
curl -s ".../agents/{id}" | jq '.capabilities | length == 2'
curl -s ".../agents/{id}" | jq '.capabilities[0].ref == "current_time"'
curl -s ".../agents/{id}" | jq '.capabilities[1].config.timeout_ms == 30000'
```
