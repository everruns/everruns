-- Drop messages table
-- Messages are now stored as events with type "message.*"
-- The events table is the sole source of truth for conversation data

DROP TABLE messages;
