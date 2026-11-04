-- Existing bindings stay read-only until explicitly reviewed and reconnected.
ALTER TABLE agent_github_sources
    ADD COLUMN mode TEXT NOT NULL DEFAULT 'pull' CHECK (mode IN ('pull', 'two_way')),
    ADD COLUMN local_digest TEXT,
    ADD COLUMN local_package JSONB,
    ADD COLUMN local_dirty BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN outbox JSONB;
ALTER TABLE agent_github_sources DROP CONSTRAINT agent_github_sources_status_check;
ALTER TABLE agent_github_sources ADD CONSTRAINT agent_github_sources_status_check
    CHECK (status IN ('pending', 'syncing', 'synced', 'error', 'conflict'));
