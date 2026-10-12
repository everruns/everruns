-- Retire columns whose data is now tracked as session task records.
--
-- subagent_name, subagent_task, subagent_status were written only by
-- set_subagent_metadata (deleted in this changeset).  Name and lifecycle
-- status are now readable from the session_tasks registry (task.display_name /
-- task.state).  subagent_config (from migration 007) was never read or
-- written.  parent_session_id is KEPT — it is the nesting guard used by
-- spawn_subagent to prevent deeply nested spawns.
ALTER TABLE sessions
    DROP COLUMN IF EXISTS subagent_name,
    DROP COLUMN IF EXISTS subagent_task,
    DROP COLUMN IF EXISTS subagent_status,
    DROP COLUMN IF EXISTS subagent_config;
