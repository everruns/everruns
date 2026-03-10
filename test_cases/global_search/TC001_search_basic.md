# TC001: Global Search - Basic Query

## Description

Verify that the `?search=` parameter on entity list endpoints performs case-insensitive substring matching and returns only matching results.

## Preconditions

- Control-plane running (DEV_MODE or full mode)
- At least 2 agents with distinct names exist

## Steps

1. Create two agents:
   ```bash
   curl -s -X POST http://localhost:9000/v1/agents \
     -H "Content-Type: application/json" \
     -d '{"name": "Customer Support Bot", "system_prompt": "help"}'
   curl -s -X POST http://localhost:9000/v1/agents \
     -H "Content-Type: application/json" \
     -d '{"name": "Code Reviewer", "system_prompt": "review"}'
   ```

2. Search for "customer":
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=customer"
   ```

3. Search for "CUSTOMER" (uppercase):
   ```bash
   curl -s "http://localhost:9000/v1/agents?search=CUSTOMER"
   ```

4. Search with empty string:
   ```bash
   curl -s "http://localhost:9000/v1/agents?search="
   ```

## Expected Results

- Step 2: Returns 1 agent ("Customer Support Bot")
- Step 3: Returns same result (case-insensitive)
- Step 4: Returns all agents (empty search = no filter)
