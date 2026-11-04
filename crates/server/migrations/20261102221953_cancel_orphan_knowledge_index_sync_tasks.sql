-- The Knowledge index sync job was retired in code while its durable schedule
-- kept firing every minute. No worker pool claims `knowledge_index_sync`, so
-- its tasks piled up as pending until the standalone queue reached its global
-- cap (10,000) and every other scheduled job failed to enqueue (prod,
-- 2026-11-01). The server now keeps the schedule disabled and the scheduler
-- skips a trigger while the previous task is unclaimed; this releases the
-- tasks already queued. Rows are cancelled, not deleted.
UPDATE durable_schedules
SET enabled = false
WHERE name = 'knowledge-index-sync' AND enabled;

UPDATE durable_task_queue
SET status = 'cancelled'
WHERE activity_type = 'knowledge_index_sync'
  AND workflow_id IS NULL
  AND status = 'pending';
