-- no-transaction

-- Turn startup and input provenance resolve embedded message ids. The sequence
-- index still scans a conversation for these reads; index the id itself, with
-- the legacy Event envelope fallback used by the message retriever. Leave
-- deltas and tool events out to limit write amplification on the streaming path.
CREATE INDEX CONCURRENTLY idx_events_message_id ON events (
    session_id,
    (COALESCE(data->'message'->>'id', data->'data'->'message'->>'id')),
    sequence
)
WHERE event_type IN ('input.message', 'output.message.completed');
