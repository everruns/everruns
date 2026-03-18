# TC006: Create Agent - Validation Errors

## Description

Verify that agent creation fails with appropriate errors for missing or invalid fields.

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
     -d '{"name": "No Prompt Agent"}'
   ```

3. Create agent with empty body:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{}'
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 1: HTTP status | 400 or 422 |
| Step 2: HTTP status | 400 or 422 |
| Step 3: HTTP status | 400 or 422 |
| Error response | Contains field name causing the error |
