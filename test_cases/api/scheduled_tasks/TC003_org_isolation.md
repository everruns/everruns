# TC003: Organization Isolation

## Description

Verify that schedules are isolated between organizations. Users cannot access, modify, or trigger schedules belonging to other organizations.

## Preconditions

- API server is running with authentication enabled
- Two organizations exist: Org A and Org B
- Each organization has at least one API key
- Schedule "org-a-schedule" exists in Org A

## Test Data

| Field | Value |
|-------|-------|
| Org A API Key | `key_org_a_xxx` |
| Org B API Key | `key_org_b_xxx` |
| Schedule Name | org-a-schedule |
| Schedule ID | `{schedule_id}` (from Org A) |

## Steps

### Positive: Access Own Schedule

1. Using Org A API key, GET `/v1/durable/schedules/{schedule_id}`
2. Verify 200 OK response with schedule details

### Negative: Cross-Org Read

3. Using Org B API key, GET `/v1/durable/schedules/{schedule_id}`
4. Verify 404 Not Found response (not 403)

### Negative: Cross-Org Update

5. Using Org B API key, PATCH `/v1/durable/schedules/{schedule_id}` with `{"description": "hacked"}`
6. Verify 404 Not Found response

### Negative: Cross-Org Trigger

7. Using Org B API key, POST `/v1/durable/schedules/{schedule_id}/trigger`
8. Verify 404 Not Found response

### Negative: Cross-Org Delete

9. Using Org B API key, DELETE `/v1/durable/schedules/{schedule_id}`
10. Verify 404 Not Found response

### Verify Original Unchanged

11. Using Org A API key, GET `/v1/durable/schedules/{schedule_id}`
12. Verify schedule is unchanged (description not "hacked")

## Expected Result

- Org A can fully access and manage their own schedule
- Org B receives 404 (not 403) for all cross-org operations
- This prevents enumeration attacks (attacker cannot determine if schedule exists)
- Original schedule remains unmodified after all cross-org attempts

## API Verification Script

```bash
# Org A creates schedule
SCHEDULE=$(curl -s -X POST "$API_URL/v1/durable/schedules" \
  -H "Authorization: Bearer $ORG_A_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "org-a-schedule", "cron_expression": "0 * * * *", "target": {"type": "workflow", "workflow_type": "echo", "input": {}}}')
SCHEDULE_ID=$(echo $SCHEDULE | jq -r '.id')

# Org A can access
curl -s "$API_URL/v1/durable/schedules/$SCHEDULE_ID" \
  -H "Authorization: Bearer $ORG_A_KEY" | jq '.name'
# Expected: "org-a-schedule"

# Org B cannot access (should get 404)
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
  "$API_URL/v1/durable/schedules/$SCHEDULE_ID" \
  -H "Authorization: Bearer $ORG_B_KEY")
# Expected: 404

# Org B cannot trigger
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger" \
  -H "Authorization: Bearer $ORG_B_KEY")
# Expected: 404
```
