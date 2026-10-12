-- v0.8.7: squashed migrations 008–013

----------------------------------------------------------------------
-- 008: Harness inheritance
----------------------------------------------------------------------

ALTER TABLE harnesses
ADD COLUMN parent_harness_id UUID REFERENCES harnesses(id) ON DELETE RESTRICT;

CREATE INDEX idx_harnesses_parent_harness_id ON harnesses(parent_harness_id);

----------------------------------------------------------------------
-- 009: Encrypt app channel credentials (EVE-158)
----------------------------------------------------------------------

ALTER TABLE apps ADD COLUMN channel_config_encrypted BYTEA;

----------------------------------------------------------------------
-- 010: Session-level client hints (EVE-161)
----------------------------------------------------------------------

ALTER TABLE sessions ADD COLUMN hints JSONB;

----------------------------------------------------------------------
-- 011: CLI authentication
----------------------------------------------------------------------

ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}';

CREATE TABLE cli_auth_sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    state TEXT NOT NULL UNIQUE,
    exchange_code TEXT NOT NULL UNIQUE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    redirect_port INT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cli_auth_sessions_state ON cli_auth_sessions(state);
CREATE INDEX idx_cli_auth_sessions_exchange_code ON cli_auth_sessions(exchange_code);
CREATE INDEX idx_cli_auth_sessions_expires_at ON cli_auth_sessions(expires_at);

----------------------------------------------------------------------
-- 012: Session Git Objects & Refs
----------------------------------------------------------------------

CREATE TABLE session_git_objects (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    oid BYTEA NOT NULL,
    obj_type SMALLINT NOT NULL,
    size BIGINT NOT NULL,
    data BYTEA NOT NULL,
    PRIMARY KEY (session_id, oid),
    CONSTRAINT chk_git_object_oid_len CHECK (octet_length(oid) = 20),
    CONSTRAINT chk_git_object_type CHECK (obj_type IN (1, 2, 3, 4))
);

CREATE INDEX idx_session_git_objects_session
    ON session_git_objects (session_id);

CREATE TABLE session_git_refs (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    target BYTEA NOT NULL,
    is_symbolic BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (session_id, name),
    CONSTRAINT chk_git_ref_target_len CHECK (is_symbolic = true OR octet_length(target) = 20)
);

CREATE INDEX idx_session_git_refs_session
    ON session_git_refs (session_id);

----------------------------------------------------------------------
-- 013: Agent identities
----------------------------------------------------------------------

CREATE TABLE agent_identities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    avatar_url TEXT,
    locale TEXT,
    timezone TEXT,
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_identities_org_id ON agent_identities(org_id);
CREATE INDEX idx_agent_identities_org_status ON agent_identities(org_id, status);
CREATE INDEX idx_agent_identities_org_name ON agent_identities(org_id, name);

CREATE TRIGGER update_agent_identities_updated_at BEFORE UPDATE ON agent_identities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

ALTER TABLE sessions
    ADD COLUMN agent_identity_id UUID REFERENCES agent_identities(id);

CREATE INDEX idx_sessions_agent_identity_id ON sessions(agent_identity_id);
CREATE INDEX idx_sessions_org_agent_identity_id ON sessions(org_id, agent_identity_id);

ALTER TABLE apps
    ADD COLUMN agent_identity_id UUID REFERENCES agent_identities(id);

CREATE INDEX idx_apps_agent_identity_id ON apps(agent_identity_id);
CREATE INDEX idx_apps_org_agent_identity_id ON apps(org_id, agent_identity_id);
