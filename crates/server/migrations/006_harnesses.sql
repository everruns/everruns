-- Migration: Add harnesses table and link to sessions
--
-- Harness defines base rules and capabilities for sessions.
-- Hierarchy: Harness (required) → Agent (optional) → Session

-- Harnesses table (same structure as agents, no usage tracking)
CREATE TABLE harnesses (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_harnesses_org_id ON harnesses(org_id);

-- Harness capabilities junction table
CREATE TABLE harness_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    harness_id UUID NOT NULL REFERENCES harnesses(id) ON DELETE CASCADE,
    capability_id VARCHAR(50) NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    config JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(harness_id, capability_id)
);
CREATE INDEX idx_harness_caps_harness_id ON harness_capabilities(harness_id);

-- Add harness_id to sessions (nullable for existing data), make agent_id optional
ALTER TABLE sessions ADD COLUMN harness_id UUID REFERENCES harnesses(id);
ALTER TABLE sessions ALTER COLUMN agent_id DROP NOT NULL;
