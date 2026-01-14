-- MCP Servers table
-- Stores configuration for Model Context Protocol (MCP) servers
-- Currently supports only HTTP (Streamable HTTP) transport type

-- ============================================
-- MCP Servers
-- ============================================

CREATE TABLE mcp_servers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- URL of the MCP server endpoint (e.g., https://mcp.atlassian.com/v1/mcp)
    url TEXT NOT NULL,
    -- Transport type: currently only 'http' is supported
    transport_type VARCHAR(50) NOT NULL DEFAULT 'http' CHECK (transport_type IN ('http')),
    -- Status for lifecycle management
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    -- Optional API key for authentication (encrypted)
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    -- Additional headers as JSON (e.g., for custom authentication)
    headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- MCP server-specific settings
    settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Unique constraint on name to prevent duplicates
CREATE UNIQUE INDEX idx_mcp_servers_name ON mcp_servers(name);
CREATE INDEX idx_mcp_servers_status ON mcp_servers(status);

CREATE TRIGGER update_mcp_servers_updated_at BEFORE UPDATE ON mcp_servers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
