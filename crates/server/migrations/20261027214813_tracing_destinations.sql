-- Customer tracing is independent of platform telemetry and personal grants.
CREATE TABLE tracing_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    secrets_encrypted BYTEA NOT NULL,
    UNIQUE (org_id, id)
);
CREATE TABLE tracing_destinations (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    connection_id UUID NOT NULL,
    config JSONB NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    last_accepted_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    last_test_at TIMESTAMPTZ,
    last_error TEXT,
    dropped_events BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY (org_id, connection_id) REFERENCES tracing_connections(org_id, id),
    UNIQUE (org_id, id)
);
CREATE INDEX tracing_destinations_org ON tracing_destinations(org_id);
CREATE TABLE tracing_policies (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    -- NULL is organization defaults. Agent policies remain outside agent packages.
    agent_id UUID REFERENCES agents(id) ON DELETE CASCADE,
    policy JSONB NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE NULLS NOT DISTINCT (org_id, agent_id)
);
