-- Everruns v0.8.3
-- Squashed migration for changes between v0.8.0 and v0.8.3
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
--
-- Includes:
-- - Structured audit log for security-relevant events (TM-OBS-007)
-- - Apps: deployable units binding Harness + Agent to distribution channels

-- ============================================
-- Audit Logs (TM-OBS-007)
-- ============================================
-- Covers: authentication, API key management, OAuth, org membership changes

CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    -- Actor: who performed the action (NULL for unauthenticated attempts)
    actor_id UUID REFERENCES users(id),
    -- Structured event type: domain.action.outcome (e.g. auth.login.success)
    event_type TEXT NOT NULL,
    -- Client IP address (may be proxy-forwarded)
    ip_address TEXT,
    -- Freeform metadata (e.g. user_agent, provider, target resource)
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Query patterns: by org (list), by actor (investigation), by type (alerting), by time (retention)
CREATE INDEX IF NOT EXISTS idx_audit_logs_org_created ON audit_logs (org_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_actor ON audit_logs (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_event_type ON audit_logs (event_type, created_at DESC);

-- ============================================
-- Apps (specs/apps.md)
-- ============================================
-- Deployable units binding Harness + Agent to distribution channels

CREATE TABLE apps (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- Required references to harness and agent
    harness_id UUID NOT NULL REFERENCES harnesses(id),
    agent_id UUID NOT NULL REFERENCES agents(id),
    -- Channel configuration
    channel_type VARCHAR(50) NOT NULL CHECK (channel_type IN ('slack')),
    channel_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Lifecycle: draft -> published -> archived -> deleted
    status VARCHAR(50) NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived', 'deleted')),
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    -- Unique public_id per org
    CONSTRAINT apps_public_id_format CHECK (public_id ~ '^app_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_apps_org_public_id ON apps(org_id, public_id);
CREATE INDEX idx_apps_org_id ON apps(org_id);
CREATE INDEX idx_apps_org_status ON apps(org_id, status);

CREATE TRIGGER update_apps_updated_at BEFORE UPDATE ON apps
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
