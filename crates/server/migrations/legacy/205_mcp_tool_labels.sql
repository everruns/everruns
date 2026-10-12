-- Saved per-tool risk labels on organization MCP servers.
--
-- A person marks an MCP tool `read_only` (never asks for approval in the
-- normal approval mode) or `changes` (always asks). The label wins over the
-- tool's own annotations and is keyed by server and tool name, so it survives
-- tool refreshes. `suggested_label` holds a later automated suggestion that a
-- person must confirm; nothing writes it yet.
--
-- A dedicated table, not a key in `mcp_servers.settings`: settings round-trip
-- through typed settings, which would drop an unknown key (see migration 193).
-- See knowledge/integrations/mcp-servers.md ("Tool risk labels").

CREATE TABLE mcp_tool_labels (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    mcp_server_id UUID NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    label TEXT CHECK (label IN ('read_only', 'changes')),
    suggested_label TEXT CHECK (suggested_label IN ('read_only', 'changes')),
    -- Audit only: the label belongs to the server and outlives the person.
    set_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (mcp_server_id, tool_name)
);

CREATE INDEX idx_mcp_tool_labels_org_server ON mcp_tool_labels(org_id, mcp_server_id);

CREATE TRIGGER update_mcp_tool_labels_updated_at BEFORE UPDATE ON mcp_tool_labels
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
