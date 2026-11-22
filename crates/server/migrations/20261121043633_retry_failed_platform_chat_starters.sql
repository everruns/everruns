-- A failed cell create soft-deletes its control-plane record. A deleted
-- starter cannot open, and must not reserve Main chat on the next attempt.
-- Archived starters still reserve it.
CREATE UNIQUE INDEX idx_sessions_live_platform_chat_starter_owner
    ON sessions (org_id, owner_principal_id)
    WHERE deleted_at IS NULL AND tags @> ARRAY['platform-chat-starter']::text[];

DROP INDEX idx_sessions_platform_chat_starter_owner;
ALTER INDEX idx_sessions_live_platform_chat_starter_owner
    RENAME TO idx_sessions_platform_chat_starter_owner;
