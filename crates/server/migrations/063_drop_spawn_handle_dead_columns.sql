-- Drop dead-weight metadata columns from subagent_spawn_handles.
--
-- subagent_name and subagent_task were written at claim time but never read:
-- the EVE-535 dedup/claim/reattach logic keys on (parent_session_id,
-- tool_call_id) and only reads id, child_session_id, status, terminal_status,
-- terminal_result, claim_token. The columns are pure storage overhead.
ALTER TABLE subagent_spawn_handles
    DROP COLUMN IF EXISTS subagent_name,
    DROP COLUMN IF EXISTS subagent_task;
