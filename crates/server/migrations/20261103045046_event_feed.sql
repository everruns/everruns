-- Event listener feed (knowledge/execution/event-listeners.md).
--
-- Each event row records the id of the transaction that wrote it. A consumer
-- reads events between its cursor and the oldest transaction still running,
-- so a late commit is read on a later pass and never skipped.
--
-- The column is added without a default first, so existing rows stay NULL and
-- the table is not rewritten; the default then applies to new rows only.
-- Rows written before this migration are never read by the feed: a consumer
-- starts at the read point of its first pass.
ALTER TABLE events ADD COLUMN xact_id xid8;
ALTER TABLE events ALTER COLUMN xact_id SET DEFAULT pg_current_xact_id();

-- One read point per consumer and partition: every event of the partition
-- with a transaction id below `next_xact_id` was handed to the consumer.
CREATE TABLE event_feed_cursors (
    consumer text NOT NULL,
    partition smallint NOT NULL,
    next_xact_id xid8 NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, partition)
);

-- The last sequence of each session handed to a consumer. Keeps a session's
-- events in order when a transaction that took its id earlier commits after a
-- later one (the feed then delivers the late one early, and skips it when the
-- read point reaches it).
CREATE TABLE event_feed_sessions (
    consumer text NOT NULL,
    session_id uuid NOT NULL,
    delivered_sequence integer NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, session_id)
);
