-- Session participants: durable membership model for agent/user collaboration.
--
-- `sessions.agent_id` remains the denormalized host-agent pointer for existing
-- reads. This table is the canonical participant set used by the follow-on
-- API/runtime slices.

CREATE TABLE session_participants (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'user')),
    agent_id UUID REFERENCES agents(id) ON DELETE CASCADE,
    agent_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    principal_id UUID NOT NULL REFERENCES principals(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('host', 'member')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    left_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT session_participants_agent_shape CHECK (
        (kind = 'agent' AND agent_id IS NOT NULL)
        OR (kind = 'user' AND agent_id IS NULL AND agent_version_id IS NULL)
    ),
    CONSTRAINT session_participants_host_agent CHECK (
        role <> 'host' OR kind = 'agent'
    ),
    CONSTRAINT session_participants_interval CHECK (
        left_at IS NULL OR left_at >= joined_at
    )
);

CREATE UNIQUE INDEX session_participants_one_active_host_idx
    ON session_participants(session_id)
    WHERE kind = 'agent' AND role = 'host' AND left_at IS NULL;

CREATE INDEX idx_session_participants_org_session
    ON session_participants(org_id, session_id, joined_at);

CREATE INDEX idx_session_participants_agent
    ON session_participants(org_id, agent_id)
    WHERE agent_id IS NOT NULL;

CREATE INDEX idx_session_participants_principal
    ON session_participants(org_id, principal_id);

CREATE INDEX idx_session_participants_session_left_at
    ON session_participants(session_id, left_at);

CREATE TRIGGER update_session_participants_updated_at BEFORE UPDATE ON session_participants
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

INSERT INTO session_participants (
    id, org_id, session_id, kind, agent_id, agent_version_id,
    principal_id, role, joined_at, created_at, updated_at
)
SELECT
    uuidv7(),
    org_id,
    id,
    'agent',
    agent_id,
    agent_version_id,
    owner_principal_id,
    'host',
    created_at,
    created_at,
    updated_at
FROM sessions
WHERE agent_id IS NOT NULL;

INSERT INTO session_participants (
    id, org_id, session_id, kind, agent_id, agent_version_id,
    principal_id, role, joined_at, created_at, updated_at
)
SELECT
    uuidv7(),
    org_id,
    id,
    'user',
    NULL,
    NULL,
    owner_principal_id,
    'member',
    created_at,
    created_at,
    updated_at
FROM sessions;

COMMENT ON TABLE session_participants IS
    'Durable membership rows for users and agents participating in a session.';
COMMENT ON COLUMN session_participants.kind IS
    'Participant actor kind: agent or user.';
COMMENT ON COLUMN session_participants.role IS
    'Participant role within the session. Host is reserved for the active agent.';
COMMENT ON COLUMN session_participants.agent_version_id IS
    'Immutable agent version captured for agent participants when known.';
