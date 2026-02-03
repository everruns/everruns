# TC006: Schedule API CRUD Operations

## Description

Verify all CRUD operations work correctly via the REST API.

## Preconditions

- API server is running
- `curl` or similar HTTP client available

## Test Data

| Field | Value |
|-------|-------|
| API Base URL | http://localhost:9000 |
| Schedule Name | api-test-schedule |

## Steps

### 1. Create Schedule (POST)

```bash
curl -X POST "$API_URL/v1/durable/schedules" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "api-test-schedule",
    "description": "Created via API",
    "cron_expression": "0 */6 * * *",
    "timezone": "America/New_York",
    "target": {
      "type": "workflow",
      "workflow_type": "echo",
      "input": {"source": "api-test"}
    },
    "enabled": true,
    "max_concurrent": 2
  }'
```

**Expected**: 201 Created, response contains `id`, `next_trigger_at`

### 2. Get Schedule (GET)

```bash
curl "$API_URL/v1/durable/schedules/$SCHEDULE_ID"
```

**Expected**: 200 OK, all fields match creation request

### 3. List Schedules (GET)

```bash
curl "$API_URL/v1/durable/schedules"
```

**Expected**: 200 OK, `data` array contains the created schedule, `total` >= 1

### 4. Update Schedule (PATCH)

```bash
curl -X PATCH "$API_URL/v1/durable/schedules/$SCHEDULE_ID" \
  -H "Content-Type: application/json" \
  -d '{
    "description": "Updated description",
    "cron_expression": "0 */12 * * *"
  }'
```

**Expected**: 200 OK, `description` and `cron_expression` updated, `next_trigger_at` recalculated

### 5. Pause Schedule (POST)

```bash
curl -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/pause"
```

**Expected**: 200 OK, `enabled: false`, `next_trigger_at: null`

### 6. Resume Schedule (POST)

```bash
curl -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/resume"
```

**Expected**: 200 OK, `enabled: true`, `next_trigger_at` populated

### 7. Manual Trigger (POST)

```bash
curl -X POST "$API_URL/v1/durable/schedules/$SCHEDULE_ID/trigger"
```

**Expected**: 200 OK, response contains `execution_id`

### 8. List Executions (GET)

```bash
curl "$API_URL/v1/durable/schedules/$SCHEDULE_ID/executions"
```

**Expected**: 200 OK, `data` array contains at least one execution from step 7

### 9. Get Execution (GET)

```bash
curl "$API_URL/v1/durable/executions/$EXECUTION_ID"
```

**Expected**: 200 OK, execution details with status and timing

### 10. Delete Schedule (DELETE)

```bash
curl -X DELETE "$API_URL/v1/durable/schedules/$SCHEDULE_ID"
```

**Expected**: 204 No Content

### 11. Verify Deletion (GET)

```bash
curl "$API_URL/v1/durable/schedules/$SCHEDULE_ID"
```

**Expected**: 404 Not Found

## Expected Result

All API operations return expected status codes and response bodies. The schedule lifecycle (create, read, update, pause, resume, trigger, delete) works correctly.

## Error Cases to Verify

| Request | Expected |
|---------|----------|
| POST with duplicate name | 409 Conflict |
| POST with invalid cron | 400 Bad Request |
| GET non-existent ID | 404 Not Found |
| DELETE non-existent ID | 404 Not Found |
| PATCH with invalid field | 400 Bad Request |
