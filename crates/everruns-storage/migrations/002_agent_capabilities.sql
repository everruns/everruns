-- Agent Capabilities Schema (v0.2.1)
-- Decision: Capabilities are stored as a junction table with ordering
-- Design: Capabilities are external to Agent Loop - resolved at API layer to configure AgentConfig

-- ============================================
-- Agent Capabilities (junction table)
-- ============================================

CREATE TABLE agent_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    -- Capability ID is a string enum (noop, current_time, research, sandbox, file_system)
    capability_id VARCHAR(50) NOT NULL CHECK (capability_id IN ('noop', 'current_time', 'research', 'sandbox', 'file_system')),
    -- Position determines the order in the capability chain (lower = earlier)
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Each agent can have each capability only once
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);
