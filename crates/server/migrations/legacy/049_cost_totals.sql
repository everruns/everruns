-- Split per-generation cost into actual vs estimated + denormalized cost totals.
--
-- Migration 048 added `llm_generations.cost_usd` to hold the provider's
-- authoritative inline cost (e.g. OpenRouter's usage.cost). Rename it to
-- `actual_cost_usd` and add a parallel `estimated_cost_usd` that holds the
-- price-table estimate, computed for every generation that has profile cost
-- data. Keeping both lets a real charge stay distinguishable from an estimate
-- and lets estimate-vs-actual drift be audited.
ALTER TABLE llm_generations
    RENAME COLUMN cost_usd TO actual_cost_usd;

ALTER TABLE llm_generations
    ADD COLUMN estimated_cost_usd DOUBLE PRECISION;

COMMENT ON COLUMN llm_generations.actual_cost_usd IS
    'Provider-reported actual cost in USD (e.g. OpenRouter usage.cost); NULL when the provider does not report one.';
COMMENT ON COLUMN llm_generations.estimated_cost_usd IS
    'Price-table estimate of cost in USD from the model profile; NULL when there is no profile cost data.';

-- Denormalized cost totals parallel to the existing token totals, so per-session
-- and per-agent spend (and dashboard aggregates) can be read without summing
-- llm_generations. Maintained incrementally by the usage-tracking listener.
-- Three figures are kept: actual, estimated, and a best-effort total where the
-- actual cost takes priority over the estimate per generation.
ALTER TABLE sessions
    ADD COLUMN total_actual_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN total_estimated_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN total_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0;

ALTER TABLE agents
    ADD COLUMN total_actual_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN total_estimated_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN total_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0;

COMMENT ON COLUMN sessions.total_actual_cost_usd IS
    'Denormalized: sum of actual_cost_usd from llm_generations in this session.';
COMMENT ON COLUMN sessions.total_estimated_cost_usd IS
    'Denormalized: sum of estimated_cost_usd from llm_generations in this session.';
COMMENT ON COLUMN sessions.total_cost_usd IS
    'Denormalized best-effort spend: sum of actual_cost_usd where present, else estimated_cost_usd, per generation.';
COMMENT ON COLUMN agents.total_actual_cost_usd IS
    'Denormalized: sum of actual_cost_usd across all sessions for this agent.';
COMMENT ON COLUMN agents.total_estimated_cost_usd IS
    'Denormalized: sum of estimated_cost_usd across all sessions for this agent.';
COMMENT ON COLUMN agents.total_cost_usd IS
    'Denormalized best-effort spend: sum of actual_cost_usd where present, else estimated_cost_usd, per generation.';
