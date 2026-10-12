-- Everruns v0.8.0
-- Squashed migration - all schema changes since v0.7.0
-- BREAKING CHANGE: Requires fresh database (drop existing _sqlx_migrations table)
--
-- Includes:
-- - images.org_id for multi-tenant isolation
-- - Event push notifications via PostgreSQL LISTEN/NOTIFY
-- - Event retention with archived_events table and archival bypass
-- - User connections (OAuth, GitHub App, API key)
-- - Per-user session pinning
-- - GitHub App installation_id support
-- - Organization member roles (owner/admin/member)
-- - User connection types (OAuth vs API key)
-- - Session-scoped task scheduling (cron + one-shot)

-- ============================================
-- Images: add org_id for multi-tenant isolation
-- ============================================

ALTER TABLE images ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1;
CREATE INDEX idx_images_org_id ON images(org_id);

-- ============================================
-- Event push notifications via PostgreSQL LISTEN/NOTIFY
-- ============================================
-- Replaces SSE polling (100-500ms intervals) with push-based notifications
-- for low-latency event delivery (<20ms). Mirrors the pattern used by
-- durable task queue (notify_task_available).
--
-- Channel: 'event_available'
-- Payload: session_id UUID (allows per-session subscriber filtering)

CREATE OR REPLACE FUNCTION notify_event_available()
RETURNS TRIGGER AS $$
BEGIN
    -- Notify with session_id as payload for subscriber filtering
    PERFORM pg_notify('event_available', NEW.session_id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger on event insert
CREATE TRIGGER event_insert_notify
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION notify_event_available();

-- ============================================
-- Event retention: archived_events and archival bypass
-- ============================================
-- Old events are moved from `events` to `archived_events`, then deleted.
-- The delete trigger is bypassed by a session variable set by the archival process.
--
-- Usage: SET LOCAL app.archival_bypass = 'true'; DELETE FROM events WHERE ...;

CREATE TABLE archived_events (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    ts TIMESTAMPTZ NOT NULL,
    context JSONB NOT NULL DEFAULT '{}',
    metadata JSONB,
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_archived_events_session ON archived_events(session_id);
CREATE INDEX idx_archived_events_ts ON archived_events(ts);

COMMENT ON TABLE archived_events IS 'Cold storage for events past retention period';

-- Replace the delete trigger to allow archival bypass
CREATE OR REPLACE FUNCTION prevent_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    -- Allow deletes when archival bypass flag is set
    IF TG_OP = 'DELETE' AND current_setting('app.archival_bypass', true) = 'true' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

-- ============================================
-- User Connections (external service accounts)
-- ============================================
-- User-scoped (not org-scoped) — token represents user's identity on external service
-- No unique constraint on (user_id, provider) — enforced in app code for future flexibility
-- Same envelope encryption as session_secrets and mcp_servers.api_key_encrypted

CREATE TABLE user_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Provider identifier: 'github', 'gitlab', 'bitbucket'
    provider VARCHAR(50) NOT NULL,
    -- Connection type: 'oauth' for OAuth flows, 'api_key' for API key connections (e.g. Daytona)
    connection_type VARCHAR(20) NOT NULL DEFAULT 'oauth',
    -- Provider-side user identity
    provider_user_id TEXT,
    provider_username TEXT,
    -- Encrypted credentials (AES-256-GCM envelope encryption)
    -- NULL when using installation-based auth (GitHub App)
    access_token_encrypted BYTEA,
    refresh_token_encrypted BYTEA,
    -- GitHub App: installation_id for minting short-lived tokens
    installation_id BIGINT,
    -- What scopes were granted (e.g., 'repo,read:user')
    scopes TEXT,
    -- NULL = no expiry (GitHub OAuth App tokens don't expire)
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_connections_user_id ON user_connections(user_id);
CREATE INDEX idx_user_connections_provider ON user_connections(user_id, provider);

-- Partial index for installation-based lookups
CREATE INDEX idx_user_connections_installation
    ON user_connections(installation_id)
    WHERE installation_id IS NOT NULL;

CREATE TRIGGER update_user_connections_updated_at
    BEFORE UPDATE ON user_connections
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Per-user pinned sessions
-- ============================================
-- Users can pin sessions for quick access. Pins are scoped per-user per-org.

CREATE TABLE pinned_sessions (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    pinned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, session_id)
);

-- Index for listing a user's pinned sessions in an org
CREATE INDEX idx_pinned_sessions_user_org ON pinned_sessions(user_id, org_id);

-- ============================================
-- Organization member roles
-- ============================================
-- Hierarchical role: owner > admin > member

ALTER TABLE organization_members
    ADD COLUMN role TEXT NOT NULL DEFAULT 'member'
    CHECK (role IN ('owner', 'admin', 'member'));

CREATE INDEX idx_organization_members_role ON organization_members(org_id, role);

-- Track who created each organization
ALTER TABLE organizations
    ADD COLUMN created_by UUID REFERENCES users(id);

-- ============================================
-- Session-scoped schedules
-- ============================================
-- Agents can schedule future work within a session.
-- When a schedule fires, a message is injected into the session triggering a turn.

CREATE TABLE session_schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    public_id TEXT NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    cron_expression TEXT,
    scheduled_at TIMESTAMPTZ,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    enabled BOOLEAN NOT NULL DEFAULT true,
    next_trigger_at TIMESTAMPTZ,
    last_triggered_at TIMESTAMPTZ,
    trigger_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT session_schedules_has_schedule CHECK (
        cron_expression IS NOT NULL OR scheduled_at IS NOT NULL
    ),
    CONSTRAINT session_schedules_public_id_format CHECK (
        public_id ~ '^sched_[0-9a-f]{32}$'
    )
);

-- Unique public_id per org
CREATE UNIQUE INDEX idx_session_schedules_org_public_id
    ON session_schedules(org_id, public_id);

-- Scheduler polling: find due schedules efficiently
CREATE INDEX idx_session_schedules_polling
    ON session_schedules (next_trigger_at)
    WHERE enabled = true AND next_trigger_at IS NOT NULL;

-- List schedules for a session
CREATE INDEX idx_session_schedules_session
    ON session_schedules (session_id, created_at DESC);

-- Count active schedules per session (for max-5 enforcement)
CREATE INDEX idx_session_schedules_active_count
    ON session_schedules (session_id)
    WHERE enabled = true;

-- ============================================
-- Optimize durable_schedules scheduler polling
-- ============================================
-- The claim_due_schedules query filters on (claimed_by IS NULL OR claimed_at < stale_threshold)
-- but existing indexes only cover (next_trigger_at) WHERE enabled = true. PostgreSQL must
-- heap-fetch every matching row to evaluate the claim columns. Adding them as INCLUDE columns
-- lets the planner filter directly from the index leaf pages before acquiring FOR UPDATE locks.
-- Also removes the duplicate idx_durable_schedules_next_trigger.

DROP INDEX IF EXISTS idx_durable_schedules_next_trigger;
DROP INDEX IF EXISTS idx_durable_schedules_polling;
CREATE INDEX idx_durable_schedules_polling
    ON durable_schedules (next_trigger_at)
    INCLUDE (claimed_by, claimed_at)
    WHERE enabled = true;
