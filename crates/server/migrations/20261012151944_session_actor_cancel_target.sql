-- session_actor_cancel_target
--
-- A cancel targets the turn it was requested for, not whatever turn runs
-- next (chaos harness: turn 1 was cancelled, its holder failed its next write
-- and its lease expired without a release, and the next holder applied the
-- still-pending cancel to the follow-up message's turn).
--
-- Decisions:
-- - `cancel_inbox_seq` is the inbox position (`inbox_seq`) when the cancel was
--   requested: the cancel targets the turn whose input is at or before it.
--   Input that arrives later starts a turn the cancel never meant, and the
--   holder that records the cancel consumes input only up to it.
-- - It is set and cleared together with `cancel_requested_at`. A pending
--   cancel without one (requested before this migration) reads as the inbox
--   position at lease time, which is how such a cancel behaved before.
-- Spec: knowledge/operations/durable-execution-engine.md ("Session actors").

ALTER TABLE session_actors
    ADD COLUMN cancel_inbox_seq BIGINT;
