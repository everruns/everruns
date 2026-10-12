-- Persisted declarative capabilities.
--
-- Declarative capabilities are org-scoped capability definitions composed from
-- bounded, data-only contributions: system prompt text, scoped MCP servers,
-- text file mounts, and skill packages. Runtime workers receive the hydrated
-- definition in capability config, so full mode does not need database access
-- to execute them. API resources use public IDs (`cap_<32-hex>`); capability
-- references use stable unique names (`declarative:<name>`).

CREATE TABLE declarative_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    name VARCHAR(120) NOT NULL,
    display_name VARCHAR(120),
    description TEXT NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled', 'archived', 'deleted')),
    definition JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    CONSTRAINT declarative_capabilities_public_id_format
        CHECK (public_id ~ '^cap_[0-9a-f]{32}$'),
    CONSTRAINT declarative_capabilities_name_length
        CHECK (octet_length(name) <= 38),
    CONSTRAINT declarative_capabilities_name_format
        CHECK (name ~ '^[a-z][a-z0-9_-]*[a-z0-9]$'),
    CONSTRAINT declarative_capabilities_display_name_length
        CHECK (display_name IS NULL OR octet_length(display_name) <= 80),
    UNIQUE(org_id, public_id),
    UNIQUE(org_id, name)
);

CREATE INDEX idx_declarative_capabilities_org_id
    ON declarative_capabilities(org_id);

CREATE INDEX idx_declarative_capabilities_status
    ON declarative_capabilities(status);

CREATE TRIGGER update_declarative_capabilities_updated_at
    BEFORE UPDATE ON declarative_capabilities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
