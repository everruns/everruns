-- Agent health check runs (specs/agent-checks.md, tier-3 behavioral checks).
--
-- A health check run is a disposable, system-generated behavioral smoke test
-- of an agent configuration. Unlike evals, there are no user-curated cases or
-- separate case-result rows: the whole run — generated cases, per-case
-- results, and the aggregate summary — is stored inline as JSONB on a single
-- row, keyed by the agent and the resolved config hash so the latest result
-- for a given config can be fetched cheaply (badges, free repeat views).

CREATE TABLE agent_health_check_runs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    -- The agent this run targets. Nullable so a run survives agent deletion
    -- for audit; the run is still scoped to the org.
    agent_id UUID,
    -- Hash of the resolved config (system prompt + capabilities + model) the
    -- run was executed against. Enables "latest result for this config".
    config_hash TEXT NOT NULL,
    model_id TEXT,
    status VARCHAR(50) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    -- Aggregate metrics (pass rate, counts, avg turns, tokens). Set on finish.
    summary JSONB,
    -- Per-case results (generated case + score + judge reason + session link).
    results JSONB,
    error_message TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    CONSTRAINT agent_health_check_runs_public_id_format
        CHECK (public_id ~ '^healthcheck_[0-9a-f]{32}$'),
    CONSTRAINT agent_health_check_runs_org_id_fk
        FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT agent_health_check_runs_agent_id_fk
        FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE SET NULL
);

CREATE INDEX idx_agent_health_check_runs_org_id ON agent_health_check_runs(org_id);
-- Fetch the latest run for a given agent + config quickly.
CREATE INDEX idx_agent_health_check_runs_agent_config
    ON agent_health_check_runs(agent_id, config_hash, created_at DESC);

CREATE TRIGGER update_agent_health_check_runs_updated_at
    BEFORE UPDATE ON agent_health_check_runs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
