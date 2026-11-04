-- trace_error_classification
-- The trace shows why a turn failed by its classified error code (out of
-- credits, rate limited, provider down, ...) and how many retries a model
-- call took. Both come from events the projection already reads; rows indexed
-- before this migration keep NULL and show the error text alone.
ALTER TABLE session_trace_turns ADD COLUMN error_code TEXT;
ALTER TABLE session_trace_steps
    ADD COLUMN retry_attempts INTEGER,
    ADD COLUMN retry_wait_ms BIGINT;
