-- Agent versions: immutable runnable config snapshots for Agents.

CREATE TABLE agent_versions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL,
    semver_major INTEGER NOT NULL DEFAULT 0,
    semver_minor INTEGER NOT NULL DEFAULT 1,
    semver_patch INTEGER NOT NULL DEFAULT 0,
    version TEXT NOT NULL,
    parent_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    source_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    created_by_principal_id UUID REFERENCES principals(id) ON DELETE SET NULL,
    change_kind TEXT NOT NULL DEFAULT 'manual'
        CHECK (change_kind IN ('manual', 'patch', 'minor', 'major', 'import', 'rollback', 'fork')),
    summary TEXT,
    config_hash TEXT NOT NULL,
    authored_config JSONB NOT NULL,
    resolved_config JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT agent_versions_public_id_format
        CHECK (public_id ~ '^agentver_[0-9a-f]{32}$'),
    UNIQUE(org_id, public_id),
    UNIQUE(agent_id, version_number),
    UNIQUE(agent_id, version)
);

CREATE INDEX idx_agent_versions_org_agent
    ON agent_versions(org_id, agent_id, version_number DESC);

CREATE INDEX idx_agent_versions_parent
    ON agent_versions(parent_version_id);

ALTER TABLE agents
    ADD COLUMN default_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    ADD COLUMN forked_from_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    ADD COLUMN forked_from_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    ADD COLUMN root_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX idx_agents_default_version_id ON agents(default_version_id);
CREATE INDEX idx_agents_forked_from_agent_id ON agents(forked_from_agent_id);
CREATE INDEX idx_agents_root_agent_id ON agents(root_agent_id);

ALTER TABLE apps
    ADD COLUMN agent_version_policy TEXT NOT NULL DEFAULT 'default'
        CHECK (agent_version_policy IN ('default', 'latest', 'pinned')),
    ADD COLUMN agent_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL;

CREATE INDEX idx_apps_agent_version_id ON apps(agent_version_id);

ALTER TABLE sessions
    ADD COLUMN agent_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    ADD COLUMN agent_config_hash TEXT;

CREATE INDEX idx_sessions_agent_version_id ON sessions(agent_version_id);

COMMENT ON TABLE agent_versions IS
    'Immutable snapshots of authored and resolved Agent configuration.';
COMMENT ON COLUMN agent_versions.authored_config IS
    'User-authored Agent config at snapshot time.';
COMMENT ON COLUMN agent_versions.resolved_config IS
    'Runtime-preview config at snapshot time, including final prompt and tools.';
COMMENT ON COLUMN apps.agent_version_policy IS
    'How App resolves its Agent version: default, latest, or pinned.';
COMMENT ON COLUMN sessions.agent_version_id IS
    'Immutable Agent version captured for this session at creation or invocation time.';
