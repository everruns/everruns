-- Everruns v0.8.6
-- Squashed migration for changes between v0.8.5 and v0.8.6
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
--
-- Includes:
-- - Notifications (generic user notifications + UI inbox)
-- - Built-in harnesses (readonly, system-managed per-org)
-- - Subagents (parent/child session delegation)
-- - Leased resources (provider-owned remote state cleanup)
-- - Installed models + organization settings
-- - Initial files (starter templates on agents/harnesses)
-- - Session locale (BCP 47 tag)
-- - Continue-as-new (workflow continuation tracking)
-- - Entity lifecycle (archive/delete tombstones)
-- - Organization harness defaults

-- ============================================
-- Notifications
-- ============================================

CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind VARCHAR(100) NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    target_type VARCHAR(50),
    target_id TEXT,
    href TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    dedupe_key TEXT,
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count > 0),
    viewed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notifications_user_created
    ON notifications(user_id, org_id, created_at DESC);
CREATE INDEX idx_notifications_user_updated
    ON notifications(user_id, org_id, updated_at DESC);
CREATE INDEX idx_notifications_unviewed
    ON notifications(user_id, org_id, viewed_at, created_at DESC);
CREATE UNIQUE INDEX idx_notifications_active_dedupe
    ON notifications(org_id, user_id, dedupe_key)
    WHERE dedupe_key IS NOT NULL AND viewed_at IS NULL;

CREATE TRIGGER update_notifications_updated_at
    BEFORE UPDATE ON notifications
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE notification_turn_requests (
    input_message_id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notification_turn_requests_user
    ON notification_turn_requests(user_id, created_at DESC);
CREATE INDEX idx_notification_turn_requests_session
    ON notification_turn_requests(session_id, created_at DESC);

CREATE OR REPLACE FUNCTION notify_notification_available()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify(
        'notification_available',
        json_build_object(
            'org_id', NEW.org_id,
            'user_id', NEW.user_id,
            'action', lower(TG_OP)
        )::text
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notifications_change_notify
    AFTER INSERT OR UPDATE ON notifications
    FOR EACH ROW
    EXECUTE FUNCTION notify_notification_available();

-- ============================================
-- Built-in harnesses
-- ============================================

ALTER TABLE harnesses ADD COLUMN is_built_in BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE harnesses SET is_built_in = TRUE WHERE id IN (
    '01933b5a-0000-7000-8000-000000000601',  -- Base
    '01933b5a-0000-7000-8000-000000000602',  -- Generic
    '01933b5a-0000-7000-8000-000000000603'   -- Platform Chat
);

-- ============================================
-- Subagents
-- ============================================

ALTER TABLE sessions ADD COLUMN parent_session_id UUID REFERENCES sessions(id) ON DELETE CASCADE;
ALTER TABLE sessions ADD COLUMN subagent_name TEXT;
ALTER TABLE sessions ADD COLUMN subagent_task TEXT;
ALTER TABLE sessions ADD COLUMN subagent_config JSONB;
ALTER TABLE sessions ADD COLUMN subagent_status VARCHAR(50)
    CHECK (subagent_status IS NULL OR subagent_status IN (
        'spawning', 'running', 'completed', 'failed', 'cancelled', 'max_iterations_reached'
    ));

CREATE UNIQUE INDEX idx_subagent_name_per_parent
    ON sessions (parent_session_id, subagent_name)
    WHERE parent_session_id IS NOT NULL;

CREATE INDEX idx_sessions_parent_session_id
    ON sessions (parent_session_id)
    WHERE parent_session_id IS NOT NULL;

COMMENT ON COLUMN sessions.parent_session_id IS 'Parent session that spawned this subagent (NULL for top-level sessions)';
COMMENT ON COLUMN sessions.subagent_name IS 'Human-readable subagent name, unique per parent ("Test Runner")';
COMMENT ON COLUMN sessions.subagent_task IS 'Original task description given to the subagent';
COMMENT ON COLUMN sessions.subagent_config IS 'Inline config: allowed_tools, model, filesystem_mode, etc.';
COMMENT ON COLUMN sessions.subagent_status IS 'Subagent lifecycle: spawning→running→completed/failed/cancelled';

CREATE TABLE subagent_results (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    parent_session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    subagent_session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    task TEXT NOT NULL,
    status VARCHAR(50) NOT NULL,
    result TEXT,
    iterations INTEGER DEFAULT 0,
    tool_calls_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_subagent_results_parent ON subagent_results(parent_session_id);
CREATE INDEX idx_subagent_results_status ON subagent_results(parent_session_id, status);

COMMENT ON TABLE subagent_results IS 'Denormalized subagent completion records for fast parent queries';

-- ============================================
-- Leased resources
-- ============================================

CREATE TABLE IF NOT EXISTS leased_resources (
    id UUID PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID REFERENCES sessions(id) ON DELETE SET NULL,
    provider TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    display_name TEXT,
    status TEXT NOT NULL CHECK (status IN ('active', 'cleaning', 'released', 'cleanup_failed')),
    owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    lease_duration_seconds INTEGER NOT NULL CHECK (lease_duration_seconds > 0),
    last_touched_at TIMESTAMPTZ NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    cleanup_started_at TIMESTAMPTZ,
    cleanup_completed_at TIMESTAMPTZ,
    cleanup_attempts INTEGER NOT NULL DEFAULT 0 CHECK (cleanup_attempts >= 0),
    last_cleanup_error TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, provider, resource_type, external_id)
);

COMMENT ON TABLE leased_resources IS 'Cross-provider leased remote resources that require eventual cleanup.';
COMMENT ON COLUMN leased_resources.lease_expires_at IS 'Absolute cleanup deadline used by the durable leased-resource cleanup schedule.';
COMMENT ON COLUMN leased_resources.owner_user_id IS 'User connection owner used to resolve the same provider credentials during cleanup.';
COMMENT ON COLUMN leased_resources.metadata IS 'Non-secret provider metadata for UI, debugging, and cleanup handlers.';

CREATE INDEX IF NOT EXISTS idx_leased_resources_session_created_at
    ON leased_resources(session_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_leased_resources_due_cleanup
    ON leased_resources(lease_expires_at ASC)
    WHERE status IN ('active', 'cleanup_failed');

CREATE INDEX IF NOT EXISTS idx_leased_resources_cleaning_started_at
    ON leased_resources(cleanup_started_at ASC)
    WHERE status = 'cleaning';

CREATE INDEX IF NOT EXISTS idx_leased_resources_owner_provider
    ON leased_resources(owner_user_id, provider);

CREATE TRIGGER update_leased_resources_updated_at
    BEFORE UPDATE ON leased_resources
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Installed models + organization settings
-- ============================================

ALTER TABLE llm_models ADD COLUMN installed BOOLEAN NOT NULL DEFAULT FALSE;

-- Mark favorite models as installed for continuity
UPDATE llm_models SET installed = TRUE WHERE is_favorite = TRUE;

CREATE TABLE organization_settings (
    org_id BIGINT PRIMARY KEY REFERENCES organizations(org_id),
    default_model_id UUID REFERENCES llm_models(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Migrate is_default data to organization_settings
INSERT INTO organization_settings (org_id, default_model_id)
SELECT DISTINCT m.org_id, m.id
FROM llm_models m
WHERE m.is_default = TRUE
ON CONFLICT (org_id) DO UPDATE SET default_model_id = EXCLUDED.default_model_id;

-- Ensure every org has a settings row
INSERT INTO organization_settings (org_id)
SELECT org_id FROM organizations
ON CONFLICT (org_id) DO NOTHING;

-- Drop is_default column and its unique index
DROP INDEX IF EXISTS idx_llm_models_default;
ALTER TABLE llm_models DROP COLUMN is_default;

CREATE INDEX idx_llm_models_installed ON llm_models(installed) WHERE installed = TRUE;

-- Organization harness defaults (default + base)
ALTER TABLE organization_settings
ADD COLUMN default_harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL,
ADD COLUMN base_harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL;

UPDATE organization_settings os
SET default_harness_id = h.id
FROM harnesses h
WHERE os.default_harness_id IS NULL
  AND h.org_id = os.org_id
  AND h.is_built_in = TRUE
  AND h.name = 'Generic';

UPDATE organization_settings os
SET base_harness_id = h.id
FROM harnesses h
WHERE os.base_harness_id IS NULL
  AND h.org_id = os.org_id
  AND h.is_built_in = TRUE
  AND h.name = 'Base';

-- ============================================
-- Initial files (starter templates)
-- ============================================

ALTER TABLE agents
    ADD COLUMN initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE harnesses
    ADD COLUMN initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

COMMENT ON COLUMN agents.initial_files IS 'Starter files copied into each new session for this agent';
COMMENT ON COLUMN harnesses.initial_files IS 'Starter files copied into each new session for this harness';

-- ============================================
-- Session locale
-- ============================================

ALTER TABLE sessions ADD COLUMN locale TEXT;

COMMENT ON COLUMN sessions.locale IS 'Optional BCP 47 locale tag for localized agent behavior (for example uk-UA)';

-- ============================================
-- Continue-as-new (workflow continuation)
-- ============================================

ALTER TABLE durable_workflow_instances
ADD COLUMN IF NOT EXISTS continued_as_new_id UUID REFERENCES durable_workflow_instances(id);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_continued_as_new_id
ON durable_workflow_instances(continued_as_new_id)
WHERE continued_as_new_id IS NOT NULL;

-- ============================================
-- Entity lifecycle (archive/delete)
-- ============================================

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
