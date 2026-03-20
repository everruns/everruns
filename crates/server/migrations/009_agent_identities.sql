-- Agent identities: virtual principals used by apps and unattended sessions.
--
-- Scope in this migration:
-- - Introduce first-class agent identities with lifecycle + preference defaults.
-- - Allow sessions and apps to bind an optional resident identity.
-- - Session schedules inherit identity implicitly through their session binding.

CREATE TABLE agent_identities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    avatar_url TEXT,
    locale TEXT,
    timezone TEXT,
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_identities_org_id ON agent_identities(org_id);
CREATE INDEX idx_agent_identities_org_status ON agent_identities(org_id, status);
CREATE INDEX idx_agent_identities_org_name ON agent_identities(org_id, name);

CREATE TRIGGER update_agent_identities_updated_at BEFORE UPDATE ON agent_identities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

ALTER TABLE sessions
    ADD COLUMN agent_identity_id UUID REFERENCES agent_identities(id);

CREATE INDEX idx_sessions_agent_identity_id ON sessions(agent_identity_id);
CREATE INDEX idx_sessions_org_agent_identity_id ON sessions(org_id, agent_identity_id);

ALTER TABLE apps
    ADD COLUMN agent_identity_id UUID REFERENCES agent_identities(id);

CREATE INDEX idx_apps_agent_identity_id ON apps(agent_identity_id);
CREATE INDEX idx_apps_org_agent_identity_id ON apps(org_id, agent_identity_id);
