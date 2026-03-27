-- ============================================
-- Evals: user-facing behavioral tests for agents
-- See specs/evals.md
-- ============================================

-- Eval: a named collection of test cases
CREATE TABLE evals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    agent_id UUID NOT NULL,
    harness_id UUID NOT NULL,
    model_override TEXT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    UNIQUE(org_id, public_id),
    CONSTRAINT evals_public_id_format CHECK (public_id ~ '^eval_[0-9a-f]{32}$'),
    CONSTRAINT evals_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT evals_agent_id_fk FOREIGN KEY (agent_id) REFERENCES agents(id),
    CONSTRAINT evals_harness_id_fk FOREIGN KEY (harness_id) REFERENCES harnesses(id)
);

CREATE INDEX idx_evals_org_id ON evals(org_id);
CREATE INDEX idx_evals_public_id ON evals(public_id);

CREATE TRIGGER update_evals_updated_at
    BEFORE UPDATE ON evals
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Eval case: a single test within an eval
CREATE TABLE eval_cases (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_id UUID NOT NULL,
    public_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    conversation JSONB NOT NULL DEFAULT '[]',
    scorers JSONB NOT NULL DEFAULT '[]',
    max_turns INTEGER,
    timeout_seconds INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(eval_id, public_id),
    CONSTRAINT eval_cases_public_id_format CHECK (public_id ~ '^evalcase_[0-9a-f]{32}$'),
    CONSTRAINT eval_cases_eval_id_fk FOREIGN KEY (eval_id) REFERENCES evals(id) ON DELETE CASCADE
);

CREATE INDEX idx_eval_cases_eval_id ON eval_cases(eval_id);

CREATE TRIGGER update_eval_cases_updated_at
    BEFORE UPDATE ON eval_cases
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Eval run: one execution of all/some cases
CREATE TABLE eval_runs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_id UUID NOT NULL,
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    model_override TEXT,
    filter_tags TEXT[],
    status VARCHAR(50) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    triggered_by TEXT NOT NULL DEFAULT 'user',
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    summary JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    CONSTRAINT eval_runs_public_id_format CHECK (public_id ~ '^evalrun_[0-9a-f]{32}$'),
    CONSTRAINT eval_runs_eval_id_fk FOREIGN KEY (eval_id) REFERENCES evals(id) ON DELETE CASCADE,
    CONSTRAINT eval_runs_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id)
);

CREATE INDEX idx_eval_runs_eval_id ON eval_runs(eval_id);
CREATE INDEX idx_eval_runs_org_id ON eval_runs(org_id);

CREATE TRIGGER update_eval_runs_updated_at
    BEFORE UPDATE ON eval_runs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Eval case result: outcome of a single case within a run
CREATE TABLE eval_case_results (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_run_id UUID NOT NULL,
    eval_case_id UUID NOT NULL,
    public_id TEXT NOT NULL,
    session_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'passed', 'failed', 'errored', 'timeout')),
    scores JSONB,
    turns INTEGER,
    latency_ms BIGINT,
    input_tokens BIGINT,
    output_tokens BIGINT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT eval_case_results_public_id_format CHECK (public_id ~ '^evalresult_[0-9a-f]{32}$'),
    CONSTRAINT eval_case_results_run_id_fk FOREIGN KEY (eval_run_id) REFERENCES eval_runs(id) ON DELETE CASCADE,
    CONSTRAINT eval_case_results_case_id_fk FOREIGN KEY (eval_case_id) REFERENCES eval_cases(id) ON DELETE CASCADE,
    CONSTRAINT eval_case_results_session_id_fk FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_eval_case_results_run_id ON eval_case_results(eval_run_id);
CREATE INDEX idx_eval_case_results_case_id ON eval_case_results(eval_case_id);

CREATE TRIGGER update_eval_case_results_updated_at
    BEFORE UPDATE ON eval_case_results
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
