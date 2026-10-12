-- no-transaction
-- The durable dashboard (`/v1/durable/sse`, `/v1/durable/tasks`) lists tasks
-- ordered by created_at. With no index on that column every SSE tick sorted
-- the whole queue (1.9 s in production, EVERRUNS-2G). Build concurrently so
-- applying this migration does not block task claims.
CREATE INDEX CONCURRENTLY idx_durable_task_queue_created_at
    ON durable_task_queue (created_at);
