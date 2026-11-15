-- monitoring_fleet
-- Fleet history for Monitoring → Timeline (knowledge/operations/monitoring.md):
-- what the worker sampler cannot see (sessions and agents in total, server
-- replicas) and what data plane v2 cells report through the feed.

-- Worker fleet (v1) and control plane, sampled by the minute job.
ALTER TABLE monitoring_minutes
    -- Sessions not archived, on the platform's own workers; once a minute.
    ADD COLUMN sessions BIGINT,
    -- Agents defined in the organization (any data plane); once a minute.
    ADD COLUMN agents INTEGER,
    -- Distinct agents with an active session, per 10-second sample.
    ADD COLUMN agents_working_sum BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN agents_working_peak INTEGER NOT NULL DEFAULT 0,
    -- Platform row only: server replicas with a fresh heartbeat.
    ADD COLUMN servers INTEGER,
    -- Data plane v2: cell minute reports folded into this minute (each cell
    -- at most once), their population summed over cells.
    ADD COLUMN dp_reports INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN dp_sessions BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN dp_running BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN dp_queued BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN dp_agents_working BIGINT NOT NULL DEFAULT 0,
    -- Data plane v2 counters from feed records: added once per stored record,
    -- never recomputed by the closing job (it reads v1 tables only).
    ADD COLUMN dp_turns_completed INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN dp_turns_failed INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN dp_turns_cancelled INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN dp_model_calls INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN dp_input_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN dp_output_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN dp_cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0;

-- One row per cell per minute: the cell's own report (`reported`) and the
-- turns its feed finished in that minute. Like monitoring_minutes, never
-- purged and without foreign keys: history outlives the cell.
CREATE TABLE monitoring_cell_minutes (
    cell TEXT NOT NULL,
    minute TIMESTAMPTZ NOT NULL,
    data_plane TEXT NOT NULL,
    -- The first report stored for the minute wins; a new leader may resend.
    reported BOOLEAN NOT NULL DEFAULT FALSE,
    leader TEXT,
    nodes INTEGER,
    nodes_answering INTEGER,
    sessions BIGINT,
    running BIGINT,
    queued BIGINT,
    resumed BIGINT,
    -- Fullest and busiest node; 0 limits are unknown.
    rss_peak_bytes BIGINT,
    memory_limit_bytes BIGINT,
    cpu_peak_cores REAL,
    cpu_limit_cores REAL,
    admit_p99_ms INTEGER,
    ship_lag_p99_ms INTEGER,
    -- Cumulative: differences between minutes are elections and moves.
    raft_term BIGINT,
    forwarded BIGINT,
    turns_completed INTEGER NOT NULL DEFAULT 0,
    turns_failed INTEGER NOT NULL DEFAULT 0,
    turns_cancelled INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (cell, minute)
);
CREATE INDEX idx_monitoring_cell_minutes_minute ON monitoring_cell_minutes (minute);
