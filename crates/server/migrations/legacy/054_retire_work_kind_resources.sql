-- Remove session_resources rows for work-shaped kinds that were dual-written
-- during the transitional period (retired in the session-tasks dual-write cleanup).
-- These kinds are now tracked exclusively via session_tasks (or platform store
-- for agent_handoff). New entries will not be created after this migration.
DELETE FROM session_resources
WHERE kind IN ('subagent', 'agent_run', 'background_run', 'agent_handoff');
