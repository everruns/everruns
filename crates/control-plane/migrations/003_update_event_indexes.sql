-- Update event indexes for the new event protocol
--
-- Event types have been simplified:
-- - message.user, message.agent (removed message.tool_call, message.tool_result)
-- - turn.started, turn.completed, turn.failed (new)
-- - tool.call_started, tool.call_completed (replaces message.tool_* for tool results)
--
-- This migration adds optimized partial indexes for common query patterns.

-- Partial index for message events (user and agent messages only)
-- Tool results are now retrieved via tool.call_completed events
CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('message.user', 'message.agent');

COMMENT ON INDEX idx_events_messages IS 'Partial index for message events (message.user, message.agent)';

-- Partial index for turn lifecycle events
CREATE INDEX idx_events_turns ON events(session_id, sequence)
WHERE event_type IN ('turn.started', 'turn.completed', 'turn.failed');

COMMENT ON INDEX idx_events_turns IS 'Partial index for turn lifecycle events';

-- Partial index for tool execution events (includes results)
CREATE INDEX idx_events_tool_calls ON events(session_id, sequence)
WHERE event_type IN ('tool.call_started', 'tool.call_completed');

COMMENT ON INDEX idx_events_tool_calls IS 'Partial index for tool execution events with results';
