-- Multitenancy: Organization-based resource isolation
-- See specs/multitenancy.md for details

-- Organizations
CREATE TABLE organizations (
    org_id BIGSERIAL PRIMARY KEY,
    public_id TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT organizations_public_id_format
        CHECK (public_id ~ '^org_[0-9a-f]{32}$')
);

CREATE INDEX idx_organizations_public_id ON organizations(public_id);

-- Organization Members
CREATE TABLE organization_members (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, user_id)
);

CREATE INDEX idx_organization_members_user_id ON organization_members(user_id);

-- Seed default organization first (required for FKs)
INSERT INTO organizations (org_id, public_id, name)
VALUES (1, 'org_00000000000000000000000000000001', 'Default Organization');

-- Reset sequence to start after seeded org
SELECT setval('organizations_org_id_seq', 1, true);

-- Add org_id to agents
ALTER TABLE agents ADD COLUMN org_id BIGINT;
UPDATE agents SET org_id = 1;
ALTER TABLE agents ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE agents ADD CONSTRAINT agents_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id);
CREATE INDEX idx_agents_org_id ON agents(org_id);

-- Add org_id to llm_providers
ALTER TABLE llm_providers ADD COLUMN org_id BIGINT;
UPDATE llm_providers SET org_id = 1;
ALTER TABLE llm_providers ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE llm_providers ADD CONSTRAINT llm_providers_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id);
CREATE INDEX idx_llm_providers_org_id ON llm_providers(org_id);

-- Add org_id to llm_models
ALTER TABLE llm_models ADD COLUMN org_id BIGINT;
UPDATE llm_models SET org_id = 1;
ALTER TABLE llm_models ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE llm_models ADD CONSTRAINT llm_models_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id);
CREATE INDEX idx_llm_models_org_id ON llm_models(org_id);

-- Add org_id to api_keys
ALTER TABLE api_keys ADD COLUMN org_id BIGINT;
UPDATE api_keys SET org_id = 1;
ALTER TABLE api_keys ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id);
CREATE INDEX idx_api_keys_org_id ON api_keys(org_id);

-- Add org_id to mcp_servers
ALTER TABLE mcp_servers ADD COLUMN org_id BIGINT;
UPDATE mcp_servers SET org_id = 1;
ALTER TABLE mcp_servers ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE mcp_servers ADD CONSTRAINT mcp_servers_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id);
CREATE INDEX idx_mcp_servers_org_id ON mcp_servers(org_id);

-- Add all existing users to default organization
INSERT INTO organization_members (org_id, user_id)
SELECT 1, id FROM users
ON CONFLICT DO NOTHING;
