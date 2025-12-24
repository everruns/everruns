-- Remove tool_call_id column from messages table
-- tool_call_id is now stored inside ToolResultContentPart in the content JSONB column

ALTER TABLE messages
    DROP COLUMN IF EXISTS tool_call_id;
