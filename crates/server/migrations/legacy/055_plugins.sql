-- Plugin marketplaces and installed plugins.
--
-- plugin_marketplaces: org-scoped catalog sources. Each marketplace has a
-- validated marketplace.json cached in `catalog` (JSONB). Sources:
--   github  — owner/repo path; synced via raw.githubusercontent
--   url     — direct HTTPS URL to a marketplace.json
--   local_path — filesystem path for dev/test only (gated by is_dev())
--
-- plugin_installs: plugins compiled from a marketplace entry and installed as
-- a `plugin:{name}` capability. `definition` is the compiled
-- DeclarativeCapabilityDefinition ready for hydration; workers never read the
-- DB for plugin config.
--
-- ID prefixes:
--   plugin_marketplaces → plgmkt_<32-hex>
--   plugin_installs     → plugin_<32-hex>
--
-- Name constraint: plugin names must fit in the VARCHAR(50) capability
-- reference columns with the `plugin:` prefix (7 bytes) → max 43 bytes.

CREATE TABLE plugin_marketplaces (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    name VARCHAR(120) NOT NULL,
    source_type VARCHAR(20) NOT NULL
        CHECK (source_type IN ('github', 'url', 'local_path')),
    source JSONB NOT NULL DEFAULT '{}'::jsonb,
    status VARCHAR(20) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    catalog JSONB,
    last_synced_at TIMESTAMPTZ,
    last_synced_sha TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT plugin_marketplaces_public_id_format
        CHECK (public_id ~ '^plgmkt_[0-9a-f]{32}$'),
    CONSTRAINT plugin_marketplaces_name_format
        CHECK (name ~ '^[a-z][a-z0-9_-]*[a-z0-9]$'),
    CONSTRAINT plugin_marketplaces_name_length
        CHECK (octet_length(name) <= 80),
    UNIQUE(org_id, public_id),
    UNIQUE(org_id, name)
);

CREATE INDEX idx_plugin_marketplaces_org_id
    ON plugin_marketplaces(org_id);

CREATE INDEX idx_plugin_marketplaces_status
    ON plugin_marketplaces(status);

CREATE TRIGGER update_plugin_marketplaces_updated_at
    BEFORE UPDATE ON plugin_marketplaces
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE plugin_installs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    name VARCHAR(50) NOT NULL,
    marketplace_id UUID REFERENCES plugin_marketplaces(id) ON DELETE SET NULL,
    source JSONB NOT NULL DEFAULT '{}'::jsonb,
    version TEXT,
    pinned_sha TEXT,
    manifest JSONB NOT NULL DEFAULT '{}'::jsonb,
    definition JSONB NOT NULL DEFAULT '{}'::jsonb,
    warnings JSONB NOT NULL DEFAULT '[]'::jsonb,
    status VARCHAR(20) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT plugin_installs_public_id_format
        CHECK (public_id ~ '^plugin_[0-9a-f]{32}$'),
    CONSTRAINT plugin_installs_name_format
        CHECK (name ~ '^[a-z][a-z0-9_-]*[a-z0-9]$'),
    CONSTRAINT plugin_installs_name_length
        CHECK (octet_length(name) <= 43),
    UNIQUE(org_id, public_id),
    UNIQUE(org_id, name)
);

CREATE INDEX idx_plugin_installs_org_id
    ON plugin_installs(org_id);

CREATE INDEX idx_plugin_installs_marketplace_id
    ON plugin_installs(marketplace_id);

CREATE INDEX idx_plugin_installs_status
    ON plugin_installs(status);

CREATE TRIGGER update_plugin_installs_updated_at
    BEFORE UPDATE ON plugin_installs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
