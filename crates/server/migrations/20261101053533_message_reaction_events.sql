-- no-transaction
-- Reactions on messages are session events (`message.reaction.added` /
-- `.removed`, knowledge/execution/message-reactions.md). Every message send
-- probes for pending reactions, so the lookup must cost one index probe in a
-- session that has none.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_events_reactions ON events (session_id, sequence)
    WHERE event_type IN ('message.reaction.added', 'message.reaction.removed');
