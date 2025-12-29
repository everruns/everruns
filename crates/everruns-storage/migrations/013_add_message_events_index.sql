-- Add partial index for efficient message event queries
-- This index supports loading messages from events table after messages table removal

CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('message.user', 'message.agent', 'message.tool_call', 'message.tool_result');

COMMENT ON INDEX idx_events_messages IS 'Partial index for querying message events efficiently';
