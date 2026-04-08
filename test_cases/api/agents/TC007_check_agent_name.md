# TC007: Check Agent Name Availability

## Description

Verify the `/v1/agents/check-name` endpoint correctly reports name availability, including format validation and exclude_id support for edit forms.

## Preconditions

- API server running (`just start-dev`)

## Steps

1. Check a name that doesn't exist:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/check-name?name=fresh-agent"
   ```

2. Create an agent:
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "taken-agent",
       "system_prompt": "Exists."
     }'
   ```
   Save `id` from response.

3. Check the taken name:
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/check-name?name=taken-agent"
   ```

4. Check the taken name with exclude_id (edit form scenario):
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/check-name?name=taken-agent&exclude_id={id}"
   ```

5. Check an invalid name format (uppercase):
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/check-name?name=Bad+Name"
   ```

6. Check a very short name (1 char):
   ```bash
   curl -s "http://localhost:9300/api/v1/agents/check-name?name=a"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 1: `available` | `true` |
| Step 3: `available` | `false` (name is taken) |
| Step 4: `available` | `true` (excluded own ID) |
| Step 5: `available` | `false` (invalid format) |
| Step 6: `available` | `true` (single char is valid) |
| All steps: HTTP status | 200 |
