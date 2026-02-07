-- Add public_id column to agents for dual-ID pattern
-- See specs/id-schema.md "Dual-ID Pattern" section
--
-- Internal: agents.id UUID (PK, used for FK references)
-- External: agents.public_id TEXT (API-facing, client-supplied or auto-generated)

ALTER TABLE agents ADD COLUMN public_id TEXT;

-- Backfill existing agents: derive public_id from internal UUID
UPDATE agents SET public_id = 'agent_' || replace(id::text, '-', '');

ALTER TABLE agents ALTER COLUMN public_id SET NOT NULL;

-- Unique per org (same public_id allowed across orgs)
CREATE UNIQUE INDEX idx_agents_org_public_id ON agents(org_id, public_id);

-- Format validation: agent_ prefix + 32 lowercase hex chars
ALTER TABLE agents ADD CONSTRAINT agents_public_id_format
    CHECK (public_id ~ '^agent_[0-9a-f]{32}$');
