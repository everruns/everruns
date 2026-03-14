-- Entity lifecycle defaults: archive and dangerous delete tombstones.
-- Adds archived_at/deleted_at and extends status enums for all user-managed building blocks.

ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS agents_status_check;
ALTER TABLE agents
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE agents
    ADD CONSTRAINT agents_status_check
    CHECK (status IN ('active', 'archived', 'deleted'));

ALTER TABLE harnesses
    DROP CONSTRAINT IF EXISTS harnesses_status_check;
ALTER TABLE harnesses
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE harnesses
    ADD CONSTRAINT harnesses_status_check
    CHECK (status IN ('active', 'archived', 'deleted'));

ALTER TABLE mcp_servers
    DROP CONSTRAINT IF EXISTS mcp_servers_status_check;
ALTER TABLE mcp_servers
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE mcp_servers
    ADD CONSTRAINT mcp_servers_status_check
    CHECK (status IN ('active', 'disabled', 'archived', 'deleted'));

ALTER TABLE skills
    DROP CONSTRAINT IF EXISTS skills_status_check;
ALTER TABLE skills
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE skills
    ADD CONSTRAINT skills_status_check
    CHECK (status IN ('active', 'disabled', 'archived', 'deleted'));

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_status_check;
ALTER TABLE apps
    ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE apps
    ADD CONSTRAINT apps_status_check
    CHECK (status IN ('draft', 'published', 'archived', 'deleted'));
