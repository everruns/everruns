CREATE TABLE mcp_service_tool_caches (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    cache_scope TEXT NOT NULL CHECK (cache_scope IN ('public', 'private')),
    credential_hash TEXT NOT NULL DEFAULT '',
    cached_tools JSONB NOT NULL,
    ttl_ms BIGINT NOT NULL CHECK (ttl_ms > 0),
    tools_cached_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, mcp_server_id, agent_id, cache_scope, credential_hash),
    CHECK (
        (cache_scope = 'public' AND credential_hash = '')
        OR (cache_scope = 'private' AND credential_hash <> '')
    )
);

CREATE INDEX idx_mcp_service_tool_caches_freshness
    ON mcp_service_tool_caches (tools_cached_at);
