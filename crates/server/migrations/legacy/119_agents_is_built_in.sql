-- Built-in agents.
--
-- Mirrors `harnesses.is_built_in` (007_v0.8.6.sql). A built-in agent is supplied
-- by the platform definition rather than authored by the org: its *definition*
-- (prompt, model, capabilities, versions) is immutable through the API, and it
-- does not count against the per-org agent limit.
--
-- Bindings an org attaches around a built-in agent — triggers, identities,
-- credentials, check rules, health checks — stay editable. A built-in agent
-- nobody can attach a trigger to is a built-in agent nobody can use.
--
-- Additive only: existing rows default to FALSE, so nothing already authored
-- becomes protected. Org bootstrap reconciles the built-in rows themselves.

ALTER TABLE agents ADD COLUMN is_built_in BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN agents.is_built_in IS
    'Platform-supplied agent: definition is immutable through the API and excluded from the per-org agent limit. Bindings (triggers, identities, credentials) remain editable.';

-- Bootstrap reconciliation looks built-in agents up per org on every startup.
CREATE INDEX idx_agents_org_built_in ON agents(org_id) WHERE is_built_in;
