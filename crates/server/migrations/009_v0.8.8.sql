-- v0.8.8: squashed migrations 009–016

----------------------------------------------------------------------
-- 009: Agent identity connections
----------------------------------------------------------------------

CREATE TABLE agent_identity_connections (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_identity_id UUID NOT NULL REFERENCES agent_identities(id) ON DELETE CASCADE,
    -- Provider identifier: 'github', 'daytona', etc.
    provider VARCHAR(50) NOT NULL,
    -- Connection type: 'oauth' or 'api_key'
    connection_type VARCHAR(20) NOT NULL DEFAULT 'api_key',
    -- Provider-side user identity
    provider_user_id TEXT,
    provider_username TEXT,
    -- Encrypted credentials (AES-256-GCM envelope encryption)
    access_token_encrypted BYTEA,
    refresh_token_encrypted BYTEA,
    -- GitHub App: installation_id for minting short-lived tokens
    installation_id BIGINT,
    -- What scopes were granted
    scopes TEXT,
    -- NULL = no expiry
    expires_at TIMESTAMPTZ,
    -- Provider-specific metadata (e.g. Deno org slug)
    provider_metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- One connection per identity per provider
    UNIQUE (agent_identity_id, provider)
);

CREATE INDEX idx_agent_identity_connections_identity
    ON agent_identity_connections(agent_identity_id);

CREATE TRIGGER update_agent_identity_connections_updated_at
    BEFORE UPDATE ON agent_identity_connections
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

----------------------------------------------------------------------
-- 010: Connection provider metadata
----------------------------------------------------------------------

ALTER TABLE user_connections
    ADD COLUMN provider_metadata JSONB;

----------------------------------------------------------------------
-- 011: Agent blueprints – blueprint-backed sessions
----------------------------------------------------------------------

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS blueprint_id TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS blueprint_config JSONB;

----------------------------------------------------------------------
-- 012: Session-level overrides
----------------------------------------------------------------------

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS system_prompt TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS initial_files JSONB NOT NULL DEFAULT '[]'::jsonb;

----------------------------------------------------------------------
-- 013: App channels table (multi-channel messaging)
----------------------------------------------------------------------

CREATE TABLE app_channels (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    -- Channel type: slack, discord, etc.
    channel_type VARCHAR(50) NOT NULL CHECK (channel_type IN ('slack')),
    -- Channel-specific config (JSON)
    channel_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Encrypted channel config (envelope-encrypted JSON)
    channel_config_encrypted BYTEA,
    -- Whether this channel is active
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Unique public_id globally (for webhook lookup without app context)
    UNIQUE (public_id),
    -- Enforce typed-ID format
    CONSTRAINT app_channels_public_id_format CHECK (public_id ~ '^appchan_[0-9a-f]{32}$')
);

CREATE INDEX idx_app_channels_app_id ON app_channels(app_id);

CREATE TRIGGER update_app_channels_updated_at
    BEFORE UPDATE ON app_channels
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Data migration: copy existing app channels
INSERT INTO app_channels (app_id, public_id, channel_type, channel_config, channel_config_encrypted, enabled)
SELECT
    a.id,
    'appchan_' || REPLACE(gen_random_uuid()::text, '-', ''),
    a.channel_type,
    a.channel_config,
    a.channel_config_encrypted,
    true
FROM apps a
WHERE a.status != 'deleted';

----------------------------------------------------------------------
-- 014: MCP OAuth (dynamic client registration, auth codes, refresh tokens)
----------------------------------------------------------------------

CREATE TABLE oauth_clients (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_id TEXT NOT NULL UNIQUE,
    client_secret_hash TEXT NOT NULL,
    client_name TEXT NOT NULL,
    redirect_uris JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_clients_client_id ON oauth_clients(client_id);

CREATE TABLE oauth_authorization_codes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    code_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL DEFAULT 'S256',
    scope TEXT NOT NULL DEFAULT 'mcp',
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_authorization_codes_code_hash ON oauth_authorization_codes(code_hash);
CREATE INDEX idx_oauth_authorization_codes_expires_at ON oauth_authorization_codes(expires_at);

CREATE TABLE oauth_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    token_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'mcp',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_refresh_tokens_token_hash ON oauth_refresh_tokens(token_hash);
CREATE INDEX idx_oauth_refresh_tokens_expires_at ON oauth_refresh_tokens(expires_at);

----------------------------------------------------------------------
-- 015: Evals (user-facing behavioral tests for agents)
----------------------------------------------------------------------

CREATE TABLE evals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    agent_id UUID NOT NULL,
    harness_id UUID NOT NULL,
    model_override TEXT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    UNIQUE(org_id, public_id),
    CONSTRAINT evals_public_id_format CHECK (public_id ~ '^eval_[0-9a-f]{32}$'),
    CONSTRAINT evals_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT evals_agent_id_fk FOREIGN KEY (agent_id) REFERENCES agents(id),
    CONSTRAINT evals_harness_id_fk FOREIGN KEY (harness_id) REFERENCES harnesses(id)
);

CREATE INDEX idx_evals_org_id ON evals(org_id);
CREATE INDEX idx_evals_public_id ON evals(public_id);

CREATE TRIGGER update_evals_updated_at
    BEFORE UPDATE ON evals
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE eval_cases (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_id UUID NOT NULL,
    public_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    conversation JSONB NOT NULL DEFAULT '[]',
    scorers JSONB NOT NULL DEFAULT '[]',
    max_turns INTEGER,
    timeout_seconds INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(eval_id, public_id),
    CONSTRAINT eval_cases_public_id_format CHECK (public_id ~ '^evalcase_[0-9a-f]{32}$'),
    CONSTRAINT eval_cases_eval_id_fk FOREIGN KEY (eval_id) REFERENCES evals(id) ON DELETE CASCADE
);

CREATE INDEX idx_eval_cases_eval_id ON eval_cases(eval_id);

CREATE TRIGGER update_eval_cases_updated_at
    BEFORE UPDATE ON eval_cases
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE eval_runs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_id UUID NOT NULL,
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    model_override TEXT,
    filter_tags TEXT[],
    status VARCHAR(50) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    triggered_by TEXT NOT NULL DEFAULT 'user',
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    summary JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    CONSTRAINT eval_runs_public_id_format CHECK (public_id ~ '^evalrun_[0-9a-f]{32}$'),
    CONSTRAINT eval_runs_eval_id_fk FOREIGN KEY (eval_id) REFERENCES evals(id) ON DELETE CASCADE,
    CONSTRAINT eval_runs_org_id_fk FOREIGN KEY (org_id) REFERENCES organizations(org_id)
);

CREATE INDEX idx_eval_runs_eval_id ON eval_runs(eval_id);
CREATE INDEX idx_eval_runs_org_id ON eval_runs(org_id);

CREATE TRIGGER update_eval_runs_updated_at
    BEFORE UPDATE ON eval_runs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE eval_case_results (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    eval_run_id UUID NOT NULL,
    eval_case_id UUID NOT NULL,
    public_id TEXT NOT NULL,
    session_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'passed', 'failed', 'errored', 'timeout')),
    scores JSONB,
    turns INTEGER,
    latency_ms BIGINT,
    input_tokens BIGINT,
    output_tokens BIGINT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT eval_case_results_public_id_format CHECK (public_id ~ '^evalresult_[0-9a-f]{32}$'),
    CONSTRAINT eval_case_results_run_id_fk FOREIGN KEY (eval_run_id) REFERENCES eval_runs(id) ON DELETE CASCADE,
    CONSTRAINT eval_case_results_case_id_fk FOREIGN KEY (eval_case_id) REFERENCES eval_cases(id) ON DELETE CASCADE,
    CONSTRAINT eval_case_results_session_id_fk FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_eval_case_results_run_id ON eval_case_results(eval_run_id);
CREATE INDEX idx_eval_case_results_case_id ON eval_case_results(eval_case_id);

CREATE TRIGGER update_eval_case_results_updated_at
    BEFORE UPDATE ON eval_case_results
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

----------------------------------------------------------------------
-- 016: Account deletion FK fixes
----------------------------------------------------------------------

ALTER TABLE audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_actor_id_fkey,
    ADD CONSTRAINT audit_logs_actor_id_fkey
        FOREIGN KEY (actor_id) REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE organizations
    DROP CONSTRAINT IF EXISTS organizations_created_by_fkey,
    ADD CONSTRAINT organizations_created_by_fkey
        FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL;
