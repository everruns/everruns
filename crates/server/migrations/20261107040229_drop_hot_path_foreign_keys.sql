-- Drop the foreign keys that every turn pays for.
--
-- Each row inserted into these tables made PostgreSQL look up, and key-share
-- lock, the parent `sessions` or `organizations` row: about 70 lookups per
-- turn in the stress bench, most of them on the one hot row of the running
-- session, against a budget of about 30 statements per turn for 8k agents on
-- the current database.
--
-- Decision: the parents are never removed under live writers. Sessions are
-- deleted only by `Database::delete_session`, which now deletes these rows
-- itself, subagent sessions included; organizations are deleted only to roll
-- back a provisioning that never ran a turn. Event inserts still go through
-- `event_sequences`, which keeps its key, so an event cannot be written for a
-- session that does not exist.
ALTER TABLE events DROP CONSTRAINT IF EXISTS events_session_id_fkey;
ALTER TABLE llm_generations DROP CONSTRAINT IF EXISTS llm_generations_session_id_fkey;
ALTER TABLE llm_generations DROP CONSTRAINT IF EXISTS llm_generations_org_id_fkey;
ALTER TABLE notification_turn_requests
    DROP CONSTRAINT IF EXISTS notification_turn_requests_session_id_fkey;
ALTER TABLE notification_turn_requests
    DROP CONSTRAINT IF EXISTS notification_turn_requests_org_id_fkey;
ALTER TABLE runtime_invocations DROP CONSTRAINT IF EXISTS runtime_invocations_session_id_fkey;
ALTER TABLE runtime_invocations DROP CONSTRAINT IF EXISTS runtime_invocations_org_id_fkey;
ALTER TABLE session_actor_lease_history
    DROP CONSTRAINT IF EXISTS session_actor_lease_history_session_id_fkey;
ALTER TABLE session_actor_lease_history
    DROP CONSTRAINT IF EXISTS session_actor_lease_history_org_id_fkey;
ALTER TABLE session_trace_turns DROP CONSTRAINT IF EXISTS session_trace_turns_session_id_fkey;
ALTER TABLE session_trace_steps
    DROP CONSTRAINT IF EXISTS session_trace_steps_session_id_turn_no_fkey;
ALTER TABLE reporting_outbox DROP CONSTRAINT IF EXISTS reporting_outbox_org_id_fkey;
