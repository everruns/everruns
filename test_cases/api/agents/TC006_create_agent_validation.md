# TC006: Create Agent - Validation Errors

## Description

Verify that agent creation fails with appropriate errors for missing, invalid, or duplicate fields, including addressable name format validation.

## Preconditions

- API server running (`just start-dev`)

## Steps

1. Create agent without name:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"system_prompt": "No name."}'
   ```

2. Create agent without system_prompt:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "no-prompt-agent"}'
   ```

3. Create agent with empty body:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{}'
   ```

4. Create agent with invalid name format (uppercase, spaces):
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "My Agent!", "system_prompt": "Bad name."}'
   ```

5. Create agent with consecutive hyphens:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "bad--name", "system_prompt": "Bad."}'
   ```

6. Create agent, then create another with the same name (duplicate):
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "unique-agent", "system_prompt": "First."}'
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "unique-agent", "system_prompt": "Duplicate."}'
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 1: HTTP status | 400 or 422 |
| Step 2: HTTP status | 400 or 422 |
| Step 3: HTTP status | 400 or 422 |
| Step 4: HTTP status | 400 (invalid name format) |
| Step 5: HTTP status | 400 (consecutive hyphens) |
| Step 6: First create | 201 |
| Step 6: Second create | 400 or 500 (name already taken) |
| Error response | Contains field name or reason causing the error |
