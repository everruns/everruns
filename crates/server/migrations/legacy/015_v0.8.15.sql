-- v0.8.15: scoped MCP servers
-- Squashed from: 015_scoped_mcp_servers.sql

-- ============================================
-- 015_scoped_mcp_servers: Harness/agent/session-scoped remote MCP server configs
-- These configs mirror the `mcpServers` map shape and are merged at runtime
-- with precedence harness -> agent -> session over org-scoped MCP servers.
-- ============================================

ALTER TABLE agents
    ADD COLUMN mcp_servers JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE harnesses
    ADD COLUMN mcp_servers JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE sessions
    ADD COLUMN mcp_servers JSONB NOT NULL DEFAULT '{}'::jsonb;
