-- Agent-first foundation: each agent owns its execution harness.
--
-- Backfill from the per-org built-in generic harness, with base as a
-- defensive fallback for older deployments that have not provisioned generic.

ALTER TABLE agents
    ADD COLUMN harness_id UUID REFERENCES harnesses(id);

UPDATE agents a
SET harness_id = COALESCE(
    (
        SELECT h.id
        FROM harnesses h
        WHERE h.org_id = a.org_id
          AND h.name = 'generic'
          AND h.is_built_in = TRUE
          AND h.status != 'deleted'
        LIMIT 1
    ),
    (
        SELECT h.id
        FROM harnesses h
        WHERE h.org_id = a.org_id
          AND h.name = 'base'
          AND h.is_built_in = TRUE
          AND h.status != 'deleted'
        LIMIT 1
    )
)
WHERE a.harness_id IS NULL;

ALTER TABLE agents
    ALTER COLUMN harness_id SET NOT NULL;

CREATE INDEX idx_agents_harness_id ON agents(harness_id);
