-- Structured audit log for security-relevant events (TM-OBS-007)
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
