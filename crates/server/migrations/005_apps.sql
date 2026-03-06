-- Apps: deployable units binding Harness + Agent to distribution channels
-- See specs/apps.md for design rationale

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
    -- Lifecycle: draft -> published -> draft (or archived)
    status VARCHAR(50) NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Unique public_id per org
    CONSTRAINT apps_public_id_format CHECK (public_id ~ '^app_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_apps_org_public_id ON apps(org_id, public_id);
CREATE INDEX idx_apps_org_id ON apps(org_id);
CREATE INDEX idx_apps_org_status ON apps(org_id, status);

CREATE TRIGGER update_apps_updated_at BEFORE UPDATE ON apps
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
