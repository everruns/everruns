# TC008: Max Schedules Per Organization Limit

## Description

Verify that the `max_schedules_per_org` limit (default: 100) is enforced when creating schedules.

## Preconditions

- API server is running
- Organization exists with API key
- `max_schedules_per_org` is configured (default: 100, use lower value like 5 for testing)

## Test Data

| Field | Value |
|-------|-------|
| Limit | 5 (for testing) |
| Cron Expression | 0 * * * * |
| Target Type | workflow |

## Steps

### Setup: Configure Lower Limit

1. Set `max_schedules_per_org=5` in configuration (for faster testing)

### Create Schedules Up To Limit

2. Create schedule "schedule-1" via POST `/v1/durable/schedules`
3. Verify 201 Created
4. Repeat for schedules 2, 3, 4, 5
5. Verify all return 201 Created

### Exceed Limit

6. Attempt to create schedule "schedule-6"
7. Verify 429 Too Many Requests response

### Verify Error Message

8. Check response body contains clear error message about limit

### Delete and Retry

9. Delete "schedule-1" via DELETE `/v1/durable/schedules/{id}`
10. Attempt to create "schedule-6" again
11. Verify 201 Created (now under limit)

## Expected Result

- Schedules 1-5 created successfully (201)
- Schedule 6 rejected with 429 Too Many Requests
- Error message: `"Organization has reached maximum schedule limit (5)"`
- After deleting one schedule, new creation succeeds

## API Verification Script

```bash
# Create 5 schedules (should all succeed)
for i in {1..5}; do
  HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
    "$API_URL/v1/durable/schedules" \
    -H "Authorization: Bearer $API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"schedule-$i\", \"cron_expression\": \"0 * * * *\", \"target\": {\"type\": \"workflow\", \"workflow_type\": \"echo\", \"input\": {}}}")
  echo "Schedule $i: $HTTP_CODE"
  # Expected: 201
done

# Attempt 6th schedule (should fail)
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  "$API_URL/v1/durable/schedules" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "schedule-6", "cron_expression": "0 * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}')
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | head -n -1)
echo "Schedule 6: $HTTP_CODE"
echo "Error: $(echo $BODY | jq -r '.error')"
# Expected: 429, "Organization has reached maximum schedule limit"

# Count active schedules
curl -s "$API_URL/v1/durable/schedules" -H "Authorization: Bearer $API_KEY" | jq '.total'
# Expected: 5
```

## Negative Cases

| Scenario | Expected |
|----------|----------|
| Create at limit | 429 Too Many Requests |
| Create disabled schedule at limit | 429 (disabled still counts) |
| Paused schedules count toward limit | Yes |
