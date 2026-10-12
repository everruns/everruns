-- Reporting phase 2: capability usage facts.
-- Facts carry only capability/tool attribution and counts. They must not
-- contain prompts, messages, tool arguments, or tool results.

CREATE TABLE fact_capability_usage (
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
    capability_id TEXT NOT NULL,
    capability_name_snapshot TEXT,
    capability_usage_kind TEXT NOT NULL
        CHECK (capability_usage_kind IN ('configured', 'resolved', 'exposed', 'invoked', 'effect_ran')),
    tool_name TEXT,
    usage_count BIGINT NOT NULL DEFAULT 1,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    projected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, source_key)
);

CREATE INDEX idx_fact_capability_usage_org_created
    ON fact_capability_usage(org_id, created_at);
CREATE INDEX idx_fact_capability_usage_org_day
    ON fact_capability_usage(org_id, time_bucket_day);
CREATE INDEX idx_fact_capability_usage_capability
    ON fact_capability_usage(org_id, capability_id, capability_usage_kind, created_at);
