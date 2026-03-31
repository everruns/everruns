-- Audit log domains (EVE-226)
--
-- Adds domain/action/target columns to the existing audit_logs table.
-- Existing rows default to domain='management' (auth events are management-domain).

-- New columns for structured audit domains
ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS domain TEXT NOT NULL DEFAULT 'management',
    ADD COLUMN IF NOT EXISTS action TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS target_type TEXT,
    ADD COLUMN IF NOT EXISTS target_id TEXT;

-- Composite index for domain-scoped queries (primary access pattern)
CREATE INDEX IF NOT EXISTS idx_audit_logs_org_domain_created
    ON audit_logs (org_id, domain, created_at DESC);

-- Action-specific queries (e.g. "show all member.invited events")
CREATE INDEX IF NOT EXISTS idx_audit_logs_org_action_created
    ON audit_logs (org_id, action, created_at DESC)
    WHERE action != '';
