-- MCP Server Tool Caching
-- Adds columns to cache discovered tools from MCP servers
-- This enables the "MCP as virtual capability" pattern where MCP servers
-- appear as capabilities in the capability selector.

-- Add tool caching columns to mcp_servers
ALTER TABLE mcp_servers
    ADD COLUMN cached_tools JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN tools_cached_at TIMESTAMPTZ;

-- Add comment for documentation
COMMENT ON COLUMN mcp_servers.cached_tools IS 'Cached tool definitions from MCP server tools/list endpoint';
COMMENT ON COLUMN mcp_servers.tools_cached_at IS 'Timestamp when tools were last fetched from the MCP server';
