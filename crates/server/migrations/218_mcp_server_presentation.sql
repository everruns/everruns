-- Display metadata discovered from a remote MCP server (title, icons, website).
-- Operator name and description stay on their own columns. `{}` means not yet fetched.

ALTER TABLE mcp_servers
    ADD COLUMN presentation JSONB NOT NULL DEFAULT '{}'::jsonb;

COMMENT ON COLUMN mcp_servers.presentation IS
    'Display metadata discovered from the remote MCP server. Not operator configuration.';
