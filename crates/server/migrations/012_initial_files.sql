-- Starter files on agents and harnesses.
--
-- Stored as JSONB arrays of objects:
-- [{ "path": "/foo.txt", "content": "...", "encoding": "text|base64", "is_readonly": bool }]

ALTER TABLE agents
    ADD COLUMN initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE harnesses
    ADD COLUMN initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

COMMENT ON COLUMN agents.initial_files IS 'Starter files copied into each new session for this agent';
COMMENT ON COLUMN harnesses.initial_files IS 'Starter files copied into each new session for this harness';
