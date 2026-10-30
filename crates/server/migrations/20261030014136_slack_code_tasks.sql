-- slack_code_tasks
CREATE TABLE slack_code_tasks (
    id UUID PRIMARY KEY,
    endpoint_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    team_id TEXT NOT NULL,
    origin_channel TEXT NOT NULL,
    origin_ts TEXT NOT NULL,
    requester TEXT NOT NULL,
    installation TEXT NOT NULL DEFAULT '',
    slack_channel TEXT,
    session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    stopped BOOLEAN NOT NULL DEFAULT FALSE,
    stop_at TIMESTAMPTZ,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    disabled BOOLEAN NOT NULL DEFAULT FALSE,
    cursor BIGINT NOT NULL DEFAULT 0,
    desired JSONB NOT NULL DEFAULT '{}',
    version BIGINT NOT NULL DEFAULT 0,
    published BIGINT NOT NULL DEFAULT -1,
    published_state JSONB NOT NULL DEFAULT '{}',
    lifecycle TEXT NOT NULL DEFAULT 'active',
    ready BOOLEAN NOT NULL DEFAULT FALSE,
    summary_ts TEXT,
    diff_view_id TEXT,
    next_attempt TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(endpoint_id, team_id, origin_channel, origin_ts),
    UNIQUE(endpoint_id, team_id, slack_channel)
);
CREATE UNIQUE INDEX slack_code_task_session ON slack_code_tasks(org_id,session_id) WHERE session_id IS NOT NULL;
CREATE INDEX slack_code_tasks_due ON slack_code_tasks(next_attempt) WHERE NOT disabled AND NOT archived AND NOT stopped;
CREATE TABLE slack_code_inbox (
    id UUID PRIMARY KEY,
    endpoint_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    team_id TEXT NOT NULL,
    event_key TEXT NOT NULL,
    payload JSONB NOT NULL,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    next_attempt TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(endpoint_id, team_id, event_key)
);
CREATE INDEX slack_code_inbox_due ON slack_code_inbox(next_attempt) WHERE NOT done;
-- Separate from the locked projection row: stop ingress must not wait for an
-- in-flight external request. No task FK/key-share lock is taken here.
CREATE TABLE slack_code_stops (
    id UUID NOT NULL,
    endpoint_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    team_id TEXT NOT NULL,
    slack_channel TEXT NOT NULL,
    stopped_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY(id,slack_channel)
);

CREATE INDEX slack_code_stops_channel ON slack_code_stops(endpoint_id,team_id,slack_channel,stopped_at DESC);
