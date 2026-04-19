-- Track which user created a session so Chats can scope to "my sessions".
ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS created_by UUID;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sessions_created_by_fkey'
    ) THEN
        ALTER TABLE sessions
            ADD CONSTRAINT sessions_created_by_fkey
            FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_sessions_org_created_by_created_at
    ON sessions(org_id, created_by, created_at DESC);
