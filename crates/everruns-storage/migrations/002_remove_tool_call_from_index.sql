-- Remove redundant idx_events_messages partial index
--
-- This partial index was created for message event queries, but:
-- 1. idx_events_session_sequence already covers (session_id, sequence) perfectly
-- 2. The event_type filter is a cheap row filter after index lookup
-- 3. The partial index adds maintenance overhead with minimal benefit
--
-- Additionally, message.tool_call events are now deprecated - tool calls are
-- embedded in message.agent events via ContentPart::ToolCall.

DROP INDEX IF EXISTS idx_events_messages;
