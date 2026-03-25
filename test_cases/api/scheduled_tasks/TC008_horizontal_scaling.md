# TC012: Horizontal Scaling with Multiple Scheduler Instances

## Description

Verify that multiple scheduler instances can run concurrently without duplicate triggers, race conditions, or missed schedules.

## Preconditions

- PostgreSQL database is running
- Multiple scheduler instances can be started (via Docker, multiple processes, etc.)
- Schedules exist with frequent cron expressions

## Test Data

| Field | Value |
|-------|-------|
| Scheduler Instances | 3 |
| Schedules | 100 (across multiple orgs) |
| Cron Expression | `* * * * *` (every minute) |
| Test Duration | 5 minutes |

## Steps

### Setup

1. Create 100 schedules across 10 organizations
2. All schedules use cron `* * * * *`

### Start Multiple Scheduler Instances

3. Start scheduler instance 1 (instance_id: "scheduler-1")
4. Start scheduler instance 2 (instance_id: "scheduler-2")
5. Start scheduler instance 3 (instance_id: "scheduler-3")

### Verify Instance Registration

6. Query `durable_scheduler_instances` table
7. Verify all 3 instances are registered with recent heartbeats

### Wait for Scheduler Cycles

8. Wait 5 minutes (5 cron cycles for each schedule)

### Verify No Duplicate Triggers

9. For each schedule, count executions in the 5-minute window
10. Each schedule should have exactly 5 executions (one per minute)
11. No schedule should have 6+ executions (would indicate duplicate trigger)

### Verify Load Distribution

12. Check which instance processed each execution
13. Verify work is distributed across all 3 instances

### Instance Failure Recovery

14. Kill scheduler instance 2 (simulate crash)
15. Wait 2 minutes
16. Verify schedules continue to be processed by instances 1 and 3
17. Verify no missed triggers during failover

### Instance Restart

18. Restart scheduler instance 2
19. Wait 1 minute
20. Verify instance 2 rejoins and processes work

## Expected Result

- All 3 instances register in `durable_scheduler_instances`
- Each schedule gets exactly 1 execution per minute (no duplicates)
- Work is distributed roughly evenly across instances
- Instance failure causes no duplicate or missed triggers
- `SELECT ... FOR UPDATE SKIP LOCKED` prevents concurrent claims

## Database Verification

```sql
-- Check instance registration
SELECT instance_id, last_heartbeat_at, schedules_processed
FROM durable_scheduler_instances
ORDER BY instance_id;
-- Expected: 3 rows with recent heartbeats

-- Check for duplicate executions (should be 0)
SELECT schedule_id, scheduled_at, COUNT(*) as exec_count
FROM durable_schedule_executions
WHERE created_at > NOW() - INTERVAL '5 minutes'
GROUP BY schedule_id, scheduled_at
HAVING COUNT(*) > 1;
-- Expected: 0 rows (no duplicates)

-- Check load distribution (which instance claimed each schedule)
SELECT claimed_by, COUNT(*) as claim_count
FROM durable_schedules
WHERE claimed_at > NOW() - INTERVAL '5 minutes'
GROUP BY claimed_by;
-- Expected: Roughly equal distribution across scheduler-1, scheduler-2, scheduler-3

-- Verify execution count per schedule
SELECT schedule_id, COUNT(*) as exec_count
FROM durable_schedule_executions
WHERE created_at > NOW() - INTERVAL '5 minutes'
GROUP BY schedule_id
HAVING COUNT(*) != 5;
-- Expected: 0 rows (each schedule should have exactly 5 executions)
```

## Failure Scenarios

| Scenario | Expected Behavior |
|----------|-------------------|
| Instance crashes mid-claim | Claimed schedules reclaimed after 30s timeout |
| Database connection lost | Instance retries, other instances continue |
| Network partition | Each partition processes independently (may cause duplicates - verify isolation) |
| All instances restart | Schedules resume processing, catch-up if configured |

## Performance Metrics

| Metric | Target |
|--------|--------|
| Claim contention rate | < 5% (SKIP LOCKED minimizes contention) |
| Reclaim events | < 1% of total claims |
| Load imbalance | < 20% variance between instances |
