# TC011: Fair Scheduling Across Organizations

## Description

Verify that the scheduler uses round-robin fairness across organizations, preventing one organization with many schedules from starving others.

## Preconditions

- API server is running with scheduler enabled
- Multiple organizations exist
- Database is accessible for verification

## Test Data

| Organization | Schedules | Cron |
|--------------|-----------|------|
| Org A | 50 schedules | `* * * * *` (every minute) |
| Org B | 5 schedules | `* * * * *` (every minute) |
| Org C | 1 schedule | `* * * * *` (every minute) |

## Steps

### Setup Organizations with Varying Schedule Counts

1. In Org A, create 50 schedules all with cron `* * * * *`
2. In Org B, create 5 schedules all with cron `* * * * *`
3. In Org C, create 1 schedule with cron `* * * * *`

### Wait for Scheduler Cycles

4. Wait 5 minutes (5 scheduler cycles)
5. Query execution counts per organization

### Verify Fair Distribution

6. Check that each organization gets roughly equal scheduling priority
7. Org C's single schedule should have ~5 executions (one per minute)
8. Org B's 5 schedules should have ~5 executions total (distributed)
9. Org A's 50 schedules should have ~5 executions total (distributed)

### Verify Per-Schedule Fairness Within Org

10. Within Org A, check execution distribution across schedules
11. All 50 schedules should get at least some executions over time

## Expected Result

- Round-robin ensures each org gets one schedule processed per batch
- Org C (1 schedule) is not starved by Org A (50 schedules)
- Maximum variance between organizations: < 10%
- Within an organization, schedules are processed in order of `next_trigger_at`

## Metrics to Verify

| Metric | Expectation |
|--------|-------------|
| Executions per org after 5 min | ~5 per org (not proportional to schedule count) |
| Org C execution count | >= 4 (not starved) |
| Max org vs min org variance | < 20% |

## Database Verification

```sql
-- Check execution distribution by organization (last 5 minutes)
SELECT
    o.name AS org_name,
    COUNT(*) AS execution_count,
    COUNT(DISTINCT se.schedule_id) AS unique_schedules_executed
FROM durable_schedule_executions se
JOIN organizations o ON se.org_id = o.org_id
WHERE se.created_at > NOW() - INTERVAL '5 minutes'
GROUP BY o.name
ORDER BY execution_count DESC;

-- Expected: Similar execution_count across orgs despite different schedule counts

-- Verify round-robin pattern in scheduler claims
SELECT
    org_id,
    COUNT(*) as claims,
    MIN(claimed_at) as first_claim,
    MAX(claimed_at) as last_claim
FROM durable_schedules
WHERE claimed_at > NOW() - INTERVAL '5 minutes'
GROUP BY org_id;
```

## Anti-Pattern Verification

Without fair scheduling, expected (bad) behavior:
- Org A would get 50/56 = 89% of executions
- Org B would get 5/56 = 9% of executions
- Org C would get 1/56 = 2% of executions

With fair scheduling, expected (good) behavior:
- Each org gets ~33% of scheduler attention
- Executions are distributed based on org count, not schedule count

## Scheduler Configuration

Verify scheduler is configured with fair scheduling:
```
scheduler_batch_size: 100
fair_scheduling: true  # Round-robin across orgs
```
