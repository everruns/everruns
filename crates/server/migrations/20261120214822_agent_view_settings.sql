-- agent_view_settings
CREATE TABLE agent_view_settings (
    agent_id UUID PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    view JSONB
);

CREATE TABLE agent_view_pending (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id UUID NOT NULL,
    agent_id UUID NOT NULL REFERENCES agent_view_settings(agent_id) ON DELETE CASCADE,
    settings_revision BIGINT NOT NULL,
    agent_scope BOOLEAN NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    draft JSONB,
    draft_sequence INTEGER,
    PRIMARY KEY (session_id, turn_id)
);

-- A null session is the Agent-wide snapshot.
CREATE TABLE agent_view_snapshots (
    agent_id UUID NOT NULL REFERENCES agent_view_settings(agent_id) ON DELETE CASCADE,
    session_id UUID REFERENCES sessions(id) ON DELETE CASCADE,
    settings_revision BIGINT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    turn_id UUID NOT NULL,
    publication JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (agent_id, session_id)
);

CREATE INDEX agent_view_snapshots_session_id_idx ON agent_view_snapshots (session_id)
    WHERE session_id IS NOT NULL;

CREATE TABLE agent_view_preferences (
    agent_id UUID NOT NULL REFERENCES agent_view_settings(agent_id) ON DELETE CASCADE,
    audience TEXT NOT NULL,
    viewer_id UUID NOT NULL,
    settings_revision BIGINT NOT NULL,
    layout JSONB,
    PRIMARY KEY (agent_id, audience, viewer_id)
);
