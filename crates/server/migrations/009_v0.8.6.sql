-- Everruns v0.8.6
-- Adds generic leased resources for provider-owned remote state cleanup.
--
-- Leased resources are the cross-provider control-plane primitive for anything
-- that must be cleaned after inactivity or explicit release:
-- - Daytona sandboxes
-- - Browserless persistent browsers
-- - Future provider-owned resources with lease/cleanup semantics
--
-- Design notes:
-- - `lease_expires_at` is stored directly so cleanup jobs can query due work
--   with an indexed predicate instead of recomputing policy windows.
-- - `session_id` is nullable with ON DELETE SET NULL so cleanup can still run
--   after a session row is removed.
-- - `owner_user_id` persists the provider connection owner used to create the
--   resource so cleanup can resolve the same provider identity later.

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
