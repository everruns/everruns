# TC006: Rate Limit Enforcement (Max Executions Per Hour)

## Description

Verify that the `max_executions_per_hour` rate limit (default: 1000) is enforced per organization. When exceeded, new triggers are rejected until the rate limit window resets.

## Preconditions

- API server is running with database-backed rate limiting
- Organization exists with API key
- `max_executions_per_hour` is configured (default: 1000, use lower value like 10 for testing)
- Multiple schedules exist in the organization

## Test Data

| Field | Value |
|-------|-------|
| Rate Limit | 10 (for testing) |
| Schedules | schedule-1, schedule-2 |

## Steps

### Setup

1. Set `max_executions_per_hour=10` in configuration
2. Create two schedules in the organization

### Trigger Up To Limit

3. Manually trigger schedule-1 via POST `/v1/durable/schedules/{id}/trigger`
4. Verify 200 OK with execution_id
5. Repeat steps 3-4 for a total of 10 triggers (mix of schedule-1 and schedule-2)
6. Verify all return 200 OK

### Exceed Rate Limit

7. Attempt 11th trigger on any schedule
8. Verify 429 Too Many Requests response
9. Check response includes rate limit info (remaining, reset time)

### Verify Automatic Triggers Also Blocked

10. Wait for a schedule's cron to fire
11. Verify execution is skipped (status: "rate_limited" or not created)

### Rate Limit Reset

12. Wait for next hour window (or adjust test time)
13. Trigger schedule again
14. Verify 200 OK (rate limit reset)

## Expected Result

- First 10 triggers succeed (200)
- 11th trigger rejected (429) with message: `"Rate limit exceeded: 10 executions per hour"`
- Response headers include:
  - `X-RateLimit-Limit: 10`
  - `X-RateLimit-Remaining: 0`
  - `X-RateLimit-Reset: {timestamp}`
- Automatic scheduler triggers are also blocked
- After hour window resets, triggers succeed again

## API Verification Script

```bash
# Create schedule for testing
SCHEDULE=$(curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "rate-test", "cron_expression": "0 * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}')
SCHEDULE_ID=$(echo $SCHEDULE | jq -r '.id')

# Trigger 10 times (should all succeed)
for i in {1..10}; do
  RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
    "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger" \
    -H "Authorization: Bearer $API_KEY")
  HTTP_CODE=$(echo "$RESPONSE" | tail -1)
  echo "Trigger $i: $HTTP_CODE"
  # Expected: 200
done

# 11th trigger (should fail)
RESPONSE=$(curl -s -D - -X POST \
  "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger" \
  -H "Authorization: Bearer $API_KEY" 2>&1)
echo "$RESPONSE" | grep -E "(HTTP|X-RateLimit|error)"
# Expected: 429, X-RateLimit-Remaining: 0

# Check rate limit counter (admin API if available)
curl -s "$API_URL/v1/durable/rate-limits" \
  -H "Authorization: Bearer $API_KEY" | jq
```

## Database Verification

```sql
-- Check rate limit table
SELECT * FROM durable_schedule_rate_limits
WHERE org_id = $org_id
ORDER BY window_start DESC
LIMIT 5;

-- Expected: execution_count = 10 for current hour window
```

## Cross-Organization Isolation

Verify rate limits are per-organization:
- Org A exhausts their rate limit
- Org B can still trigger schedules (separate rate limit counter)
