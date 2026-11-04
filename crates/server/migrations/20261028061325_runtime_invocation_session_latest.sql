-- no-transaction
-- Bound the retry identity veto to one session without blocking invocation writes.
CREATE INDEX CONCURRENTLY idx_runtime_invocations_session_latest
    ON runtime_invocations (session_id, input_message_id DESC);
