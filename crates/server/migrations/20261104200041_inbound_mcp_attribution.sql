-- Immutable server-derived MCP creator. Legacy sessions remain unattributed.
ALTER TABLE sessions ADD COLUMN mcp_creator JSONB;
ALTER TABLE entity_changes ADD COLUMN mcp_client JSONB;
ALTER TABLE account_entity_changes ADD COLUMN mcp_client JSONB;
