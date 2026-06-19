-- EVE-534: Forward-progress guard for durable turns.
--
-- A durable turn that crashes and gets reclaimed repeatedly without ever
-- advancing can loop (re-running reason/act, burning tokens/billing) until it
-- incidentally hits max-iterations. To defend against this poison-turn case we
-- track, per task, the progress token observed at the start of the previous
-- recovery attempt and a counter of consecutive recoveries during which the
-- token did not advance. When the counter reaches the configured threshold the
-- reclaim path seals the turn (marks the task dead -> DLQ) instead of requeuing.
--
-- The progress token is derived from durably-recorded facts (the highest
-- durable_workflow_events.sequence_num for the task's workflow), so it is stable
-- under replay and cannot be advanced by a non-progressing retry.

ALTER TABLE durable_task_queue
    -- Highest durable event sequence observed at the previous reclaim, encoded
    -- as a monotonic progress token (NULL = never reclaimed yet). Stored as
    -- BIGINT to match event sequence width.
    ADD COLUMN progress_token BIGINT,
    -- Number of consecutive stale reclaims during which progress_token did not
    -- advance. Reset to 0 whenever the token advances.
    ADD COLUMN no_progress_count INT NOT NULL DEFAULT 0;
