-- Drop dead-weight metadata columns from subagent_spawn_handles.
--
-- subagent_name and subagent_task were written at claim time but never read:
-- the EVE-535 dedup/claim/reattach logic keys on (parent_session_id,
-- tool_call_id) and only reads id, child_session_id, status, terminal_status,
-- terminal_result, claim_token. The columns are pure storage overhead.
--
-- Deploy note: the columns were NOT NULL (migration 052), so a pre-this-change
-- server binary still INSERTs them. This is a coordinated-deploy drop (matching
-- AGENTS.md "no backward compatibility for internal code") — apply with the
-- server build that stopped writing the columns. An old binary spawning a
-- subagent against the migrated schema would fail its claim INSERT until
-- redeployed; the dropped data itself was never read, so nothing else breaks.
ALTER TABLE subagent_spawn_handles
    DROP COLUMN IF EXISTS subagent_name,
    DROP COLUMN IF EXISTS subagent_task;
