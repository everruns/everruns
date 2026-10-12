-- Reporting phase 1: durable projection outbox, PostgreSQL facts, and indexes.
-- Facts are replayable projections of canonical operational tables; never use
-- them as sources of truth for enforcement, conversation replay, or balances.

CREATE TABLE reporting_outbox (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_version TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error TEXT,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT reporting_outbox_source_unique
        UNIQUE (org_id, source_type, source_id, source_version, reason)
);

CREATE INDEX idx_reporting_outbox_poll
    ON reporting_outbox(status, next_attempt_at, org_id, created_at);
CREATE INDEX idx_reporting_outbox_org_status
    ON reporting_outbox(org_id, status, next_attempt_at);

CREATE TRIGGER update_reporting_outbox_updated_at BEFORE UPDATE ON reporting_outbox
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE fact_llm_generation (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_key TEXT NOT NULL,
    session_id UUID,
    turn_id TEXT,
    user_id UUID,
    principal_id UUID,
    agent_id UUID,
    agent_name_snapshot TEXT,
    harness_id UUID,
    harness_name_snapshot TEXT,
    app_id UUID,
    blueprint_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    time_bucket_day DATE NOT NULL,
    model TEXT,
    provider TEXT,
    finish_reason TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT NOT NULL DEFAULT 0,
    total_tokens BIGINT NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    time_to_first_token_ms BIGINT NOT NULL DEFAULT 0,
    success_count BIGINT NOT NULL DEFAULT 1,
    error_count BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_llm_generation_org_created
    ON fact_llm_generation(org_id, created_at);
CREATE INDEX idx_fact_llm_generation_org_day
    ON fact_llm_generation(org_id, time_bucket_day);
CREATE INDEX idx_fact_llm_generation_model
    ON fact_llm_generation(org_id, model, provider, created_at);

CREATE TABLE fact_tool_call (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_key TEXT NOT NULL,
    session_id UUID,
    turn_id TEXT,
    user_id UUID,
    principal_id UUID,
    agent_id UUID,
    agent_name_snapshot TEXT,
    harness_id UUID,
    harness_name_snapshot TEXT,
    app_id UUID,
    blueprint_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    time_bucket_day DATE NOT NULL,
    tool_call_id TEXT,
    tool_name TEXT,
    tool_display_name_snapshot TEXT,
    tool_status TEXT,
    capability_id TEXT,
    capability_name_snapshot TEXT,
    call_count BIGINT NOT NULL DEFAULT 1,
    success_count BIGINT NOT NULL DEFAULT 0,
    error_count BIGINT NOT NULL DEFAULT 0,
    timeout_count BIGINT NOT NULL DEFAULT 0,
    cancelled_count BIGINT NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_tool_call_org_created
    ON fact_tool_call(org_id, created_at);
CREATE INDEX idx_fact_tool_call_org_day
    ON fact_tool_call(org_id, time_bucket_day);
CREATE INDEX idx_fact_tool_call_tool
    ON fact_tool_call(org_id, tool_name, tool_status, created_at);

CREATE TABLE fact_turn (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_key TEXT NOT NULL,
    session_id UUID,
    turn_id TEXT,
    user_id UUID,
    principal_id UUID,
    agent_id UUID,
    agent_name_snapshot TEXT,
    harness_id UUID,
    harness_name_snapshot TEXT,
    app_id UUID,
    blueprint_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    time_bucket_day DATE NOT NULL,
    turn_status TEXT,
    turn_count BIGINT NOT NULL DEFAULT 1,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    iterations BIGINT NOT NULL DEFAULT 0,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT NOT NULL DEFAULT 0,
    total_tokens BIGINT NOT NULL DEFAULT 0,
    tool_call_count BIGINT NOT NULL DEFAULT 0,
    success_count BIGINT NOT NULL DEFAULT 0,
    error_count BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_turn_org_created ON fact_turn(org_id, created_at);
CREATE INDEX idx_fact_turn_org_day ON fact_turn(org_id, time_bucket_day);
CREATE INDEX idx_fact_turn_status ON fact_turn(org_id, turn_status, created_at);

CREATE TABLE fact_session (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_key TEXT NOT NULL,
    session_id UUID,
    user_id UUID,
    principal_id UUID,
    agent_id UUID,
    agent_name_snapshot TEXT,
    harness_id UUID,
    harness_name_snapshot TEXT,
    app_id UUID,
    blueprint_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    time_bucket_day DATE NOT NULL,
    session_status TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    last_active_at TIMESTAMPTZ,
    session_count BIGINT NOT NULL DEFAULT 1,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    turn_count BIGINT NOT NULL DEFAULT 0,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT NOT NULL DEFAULT 0,
    total_tokens BIGINT NOT NULL DEFAULT 0,
    tool_call_count BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_session_org_created ON fact_session(org_id, created_at);
CREATE INDEX idx_fact_session_org_day ON fact_session(org_id, time_bucket_day);
CREATE INDEX idx_fact_session_status ON fact_session(org_id, session_status, created_at);

CREATE TABLE fact_budget_posting (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    source_key TEXT NOT NULL,
    session_id UUID,
    user_id UUID,
    principal_id UUID,
    agent_id UUID,
    agent_name_snapshot TEXT,
    harness_id UUID,
    harness_name_snapshot TEXT,
    app_id UUID,
    blueprint_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    time_bucket_day DATE NOT NULL,
    budget_id UUID,
    budget_subject_type TEXT,
    budget_subject_id TEXT,
    currency TEXT,
    meter_source TEXT,
    ref_type TEXT,
    amount DOUBLE PRECISION NOT NULL DEFAULT 0,
    debit_count BIGINT NOT NULL DEFAULT 0,
    credit_count BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_budget_posting_org_created
    ON fact_budget_posting(org_id, created_at);
CREATE INDEX idx_fact_budget_posting_org_day
    ON fact_budget_posting(org_id, time_bucket_day);
CREATE INDEX idx_fact_budget_posting_budget
    ON fact_budget_posting(org_id, budget_id, created_at);
