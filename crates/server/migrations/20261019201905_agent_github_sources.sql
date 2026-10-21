-- Source bindings belong to the control plane, never to portable packages.
CREATE TABLE agent_github_sources (
    agent_id UUID PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    generation UUID NOT NULL,
    identity_id UUID NOT NULL,
    connection_id UUID NOT NULL,
    installation_id BIGINT NOT NULL,
    configured_by UUID,
    configured_role TEXT NOT NULL,
    repository_id BIGINT NOT NULL,
    repository TEXT NOT NULL,
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    managed_channels TEXT[] NOT NULL DEFAULT '{}',
    applied_commit TEXT,
    package_digest TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'syncing', 'synced', 'error')),
    error TEXT,
    last_checked_at TIMESTAMPTZ,
    last_synced_at TIMESTAMPTZ,
    next_sync_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_token UUID,
    lease_until TIMESTAMPTZ
);
CREATE INDEX agent_github_sources_due ON agent_github_sources(next_sync_at);
CREATE INDEX agent_github_sources_repository ON agent_github_sources(org_id, repository_id, branch);
