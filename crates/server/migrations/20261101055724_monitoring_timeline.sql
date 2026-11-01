-- monitoring_timeline
-- Minute history behind Monitoring → Timeline (knowledge/operations/monitoring.md).
-- One row per organization per minute, plus a platform row (org_id = 0)
-- because peaks of different organizations do not add up. A cluster-once job
-- folds a 10-second population sample into the current minute and writes the
-- finished minute's event counters. Rows are never purged; longer ranges
-- aggregate minutes at read time. No foreign key: history outlives the
-- organization row.
CREATE TABLE monitoring_minutes (
    org_id BIGINT NOT NULL,
    minute TIMESTAMPTZ NOT NULL,
    -- Population samples folded into this minute; *_sum / samples is the average.
    samples INTEGER NOT NULL DEFAULT 0,
    active_sum BIGINT NOT NULL DEFAULT 0,
    active_peak INTEGER NOT NULL DEFAULT 0,
    model_sum BIGINT NOT NULL DEFAULT 0,
    tools_sum BIGINT NOT NULL DEFAULT 0,
    waiting_sum BIGINT NOT NULL DEFAULT 0,
    runnable_sum BIGINT NOT NULL DEFAULT 0,
    setup_sum BIGINT NOT NULL DEFAULT 0,
    runnable_peak INTEGER NOT NULL DEFAULT 0,
    leases_peak INTEGER NOT NULL DEFAULT 0,
    expired_peak INTEGER NOT NULL DEFAULT 0,
    oldest_runnable_ms BIGINT,
    organizations_peak INTEGER NOT NULL DEFAULT 0,
    -- Finished-minute counters.
    turns_started INTEGER NOT NULL DEFAULT 0,
    turns_completed INTEGER NOT NULL DEFAULT 0,
    turns_failed INTEGER NOT NULL DEFAULT 0,
    turns_cancelled INTEGER NOT NULL DEFAULT 0,
    model_calls INTEGER NOT NULL DEFAULT 0,
    tool_calls INTEGER NOT NULL DEFAULT 0,
    tool_failures INTEGER NOT NULL DEFAULT 0,
    lease_takeovers INTEGER NOT NULL DEFAULT 0,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0,
    model_latency_p95_ms INTEGER,
    -- Platform row only: registered workers and their reported load.
    workers INTEGER,
    worker_capacity INTEGER,
    worker_load_sum BIGINT,
    worker_load_peak INTEGER,
    -- {"<worker id>": {"load_sum": n, "capacity": n}} for the per-worker strip.
    worker_detail JSONB,
    -- Rebuilt from turn history rather than sampled; phases and queue are absent.
    backfilled BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (org_id, minute)
);
