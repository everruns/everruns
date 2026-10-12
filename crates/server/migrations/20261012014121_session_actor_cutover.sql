-- Hand in-flight turns from the per-step durable queue to session actors
-- (actor-based design, step 6c). Data only.
--
-- Decisions:
-- - From this release a worker runs a session's whole turn on a lease of its
--   `session_actors` row, and resumes it by folding the session's event log.
--   The per-step turn tasks (`process_input`, `reason`, `act`) and their
--   `turn_workflow` instances are no longer run by anything.
-- - Every session with a turn in flight is marked runnable: those `active`
--   and those whose `turn_workflow` is still pending or running. The first
--   worker to lease one folds its log and resumes the open turn, or starts
--   the turn its pending input asks for. Unfinished non-idempotent tool calls
--   are settled as interrupted, exactly as after a crash.
-- - The inbox cursor (`consumed_seq`) is set to the session's latest turn
--   boundary (`turn.started` or a terminal turn event): input after it is
--   either steering for the open turn or the start of the next one.
-- - A session parked waiting for tool results needs nothing: its turn is open
--   in the log, and recording the results marks the actor runnable.
-- - The old turn tasks are cancelled and their workflows marked cancelled, so
--   no stale worker or reaper can run a step of a turn an actor now owns.
--   Rows are kept (not deleted) for the queue's history and counters.

WITH turns AS (
    SELECT s.id AS session_id, s.org_id
    FROM sessions s
    WHERE s.status = 'active'
       OR EXISTS (
           SELECT 1 FROM durable_workflow_instances w
           WHERE w.id = s.id
             AND w.workflow_type = 'turn_workflow'
             AND w.status IN ('pending', 'running')
       )
)
INSERT INTO session_actors (
    session_id, org_id, runnable_since, inbox_seq, consumed_seq, next_input_seq
)
SELECT
    t.session_id,
    t.org_id,
    NOW(),
    COALESCE((SELECT MAX(e.sequence) FROM events e WHERE e.session_id = t.session_id), 0),
    COALESCE((
        SELECT MAX(e.sequence) FROM events e
        WHERE e.session_id = t.session_id
          AND e.event_type IN (
              'turn.started', 'turn.completed', 'turn.failed',
              'turn.cancelled', 'turn.sealed'
          )
    ), 0),
    0
FROM turns t
ON CONFLICT (session_id) DO UPDATE SET
    runnable_since = COALESCE(session_actors.runnable_since, NOW()),
    inbox_seq = GREATEST(session_actors.inbox_seq, EXCLUDED.inbox_seq),
    consumed_seq = GREATEST(session_actors.consumed_seq, EXCLUDED.consumed_seq);

UPDATE durable_task_queue
SET status = 'cancelled',
    last_error = 'superseded by the session actor cutover'
WHERE activity_type IN ('process_input', 'reason', 'act')
  AND status IN ('pending', 'claimed');

UPDATE durable_workflow_instances
SET status = 'cancelled',
    completed_at = NOW()
WHERE workflow_type = 'turn_workflow'
  AND status IN ('pending', 'running');
