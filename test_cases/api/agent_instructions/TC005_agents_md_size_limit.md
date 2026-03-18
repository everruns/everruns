# TC005: AGENTS.md Size Limit (32 KiB)

## Description

Verify that AGENTS.md content exceeding 32 KiB is truncated with a warning, not rejected.

## Preconditions

- Control-plane running

## Test Data

| Field | Value |
|-------|-------|
| Harness | Generic |
| AGENTS.md Size | > 32,768 bytes |

## Steps

1. Create session with Generic harness.

2. Write oversized AGENTS.md (> 32 KiB):
   ```bash
   # Generate content > 32 KiB
   CONTENT=$(python3 -c "print('x' * 40000)")
   curl -s -X POST "http://localhost:9000/v1/sessions/{session_id}/fs/AGENTS.md" \
     -H "Content-Type: application/json" \
     -d "{\"content\": \"$CONTENT\"}"
   ```

3. Send a message.

4. Verify agent responds (no crash or error).

5. Check server logs for truncation warning.

## Expected Result

| Check | Expected |
|-------|----------|
| Agent responds | Session continues normally |
| Content truncated | Server logs contain truncation warning |
| No crash | Turn completes successfully |
