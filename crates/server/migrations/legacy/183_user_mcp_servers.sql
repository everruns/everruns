-- User MCP servers: a virtual user can own MCP server records.
--
-- Decision (knowledge/integrations/user-mcp-servers.md, D1): a user-owned
-- server is an `mcp_servers` row with an owner, so OAuth discovery, client
-- registration, token refresh and URL checks reuse the catalog code. A NULL
-- owner is an organization catalog preset. Every catalog query filters on
-- `owner_virtual_user_id IS NULL`, so a person's server never appears in the
-- catalog, never resolves as `mcp:<uuid>` or `catalog:<name>`, and is never
-- visible to another person.

ALTER TABLE mcp_servers ADD COLUMN owner_virtual_user_id UUID;

-- A user server added from the catalog points at its preset and copies
-- nothing: the preset's OAuth client is reused and the grant is the owner's
-- ordinary connection to the preset. NULL for custom servers.
ALTER TABLE mcp_servers ADD COLUMN catalog_mcp_server_id UUID REFERENCES mcp_servers (id) ON DELETE CASCADE;

ALTER TABLE mcp_servers
    ADD CONSTRAINT mcp_servers_owner_virtual_user_fk
    FOREIGN KEY (org_id, owner_virtual_user_id)
    REFERENCES virtual_users (org_id, id)
    ON DELETE CASCADE;

-- Catalog names stay unique per organization among live rows (EVE-964);
-- user servers are unique per owner, so two people can both have `linear`.
DROP INDEX IF EXISTS idx_mcp_servers_org_name_live;

CREATE UNIQUE INDEX idx_mcp_servers_org_name_live
    ON mcp_servers (org_id, name)
    WHERE status IN ('active', 'disabled') AND owner_virtual_user_id IS NULL;

CREATE UNIQUE INDEX idx_mcp_servers_owner_name_live
    ON mcp_servers (org_id, owner_virtual_user_id, name)
    WHERE status IN ('active', 'disabled') AND owner_virtual_user_id IS NOT NULL;

COMMENT ON COLUMN mcp_servers.owner_virtual_user_id IS
    'Virtual user that owns this server (User MCP server); NULL for organization catalog presets';

COMMENT ON COLUMN mcp_servers.catalog_mcp_server_id IS
    'Catalog preset a user server was added from; NULL for custom user servers and for catalog rows';
