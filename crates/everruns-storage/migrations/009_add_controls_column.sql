-- Add controls column to messages table
-- Controls store runtime options like model_id and reasoning configuration

ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS controls JSONB;
