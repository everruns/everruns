-- User MCP servers: a person chooses whether a server loads on demand.
--
-- Decision (knowledge/integrations/user-mcp-servers.md, step 7a): a person's
-- servers are deferred by default, so the model sees one placeholder per server
-- until it reveals the server through tool search. A person can turn that off
-- for one of their own servers so its tools are listed from the start. A
-- dedicated column, not a key in `settings`: settings round-trip through the
-- typed OAuth/protocol settings, which would drop an unknown key.
--
-- Only user-owned rows (`owner_virtual_user_id IS NOT NULL`) read it; catalog
-- presets keep the default and ignore it (agent attachments carry their own
-- `deferred` flag).

ALTER TABLE mcp_servers ADD COLUMN deferred BOOLEAN NOT NULL DEFAULT TRUE;

COMMENT ON COLUMN mcp_servers.deferred IS
    'User MCP servers only: whether the server loads on demand (tool search reveal); ignored for catalog presets';
