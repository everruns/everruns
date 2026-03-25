# TC005: Minimum Cron Interval Validation

## Description

Verify that the `min_cron_interval_seconds` limit (default: 60) is enforced when creating or updating schedules. Cron expressions that would trigger more frequently than once per minute are rejected.

## Preconditions

- API server is running
- `min_cron_interval_seconds` is set to 60 (default)

## Test Data

| Cron Expression | Interval | Expected |
|-----------------|----------|----------|
| `* * * * *` | 60s | Allowed |
| `*/30 * * * *` | 30min | Allowed |
| `0 * * * *` | 1 hour | Allowed |
| `* * * * * *` | 1s (with seconds field) | Rejected |
| `*/30 * * * * *` | 30s | Rejected |
| `*/5 * * * * *` | 5s | Rejected |

## Steps

### Valid Cron Expressions

1. Create schedule with cron `* * * * *` (every minute)
2. Verify 201 Created

3. Create schedule with cron `*/30 * * * *` (every 30 minutes)
4. Verify 201 Created

5. Create schedule with cron `0 */6 * * *` (every 6 hours)
6. Verify 201 Created

### Invalid Cron Expressions (Too Frequent)

7. Attempt to create schedule with cron `* * * * * *` (every second)
8. Verify 400 Bad Request with error about minimum interval

9. Attempt to create schedule with cron `*/30 * * * * *` (every 30 seconds)
10. Verify 400 Bad Request

11. Attempt to create schedule with cron `*/5 * * * * *` (every 5 seconds)
12. Verify 400 Bad Request

### Update to Invalid Cron

13. Update existing schedule (from step 1) with cron `*/10 * * * * *`
14. Verify 400 Bad Request
15. Verify original schedule unchanged

## Expected Result

- Valid cron expressions (>= 60s interval) are accepted
- Invalid cron expressions (< 60s interval) return 400 Bad Request
- Error message clearly indicates: `"Cron interval (Xs) is less than minimum allowed (60s)"`
- Updates with invalid cron are rejected and don't modify existing schedule

## API Verification Script

```bash
# Valid: every minute (should succeed)
curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Content-Type: application/json" \
  -d '{"name": "valid-minute", "cron_expression": "* * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}' \
  | jq '.id'
# Expected: UUID

# Invalid: every second (should fail)
RESPONSE=$(curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Content-Type: application/json" \
  -d '{"name": "invalid-second", "cron_expression": "* * * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}')
echo $RESPONSE | jq '.error'
# Expected: "Cron interval (1s) is less than minimum allowed (60s)"

# Invalid: every 30 seconds
RESPONSE=$(curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Content-Type: application/json" \
  -d '{"name": "invalid-30s", "cron_expression": "*/30 * * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}')
echo $RESPONSE | jq '.error'
# Expected: "Cron interval (30s) is less than minimum allowed (60s)"
```

## Edge Cases

| Scenario | Expected |
|----------|----------|
| Cron `59 * * * *` (minute 59 of every hour) | Allowed (60min interval) |
| Cron `0,30 * * * *` (minutes 0 and 30) | Allowed (30min interval) |
| Malformed cron expression | 400 Bad Request (parse error) |
| Empty cron expression | 400 Bad Request |
