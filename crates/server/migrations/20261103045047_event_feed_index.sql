-- no-transaction
-- The listener feed reads one partition's events by transaction id. The
-- partition expression must match `events::feed::PARTITION_SQL` exactly, and
-- the partition count is part of it: changing the count needs a new index.
CREATE INDEX CONCURRENTLY idx_events_feed
    ON events ((get_byte(uuid_send(session_id), 15) % 8), xact_id)
    WHERE xact_id IS NOT NULL;
