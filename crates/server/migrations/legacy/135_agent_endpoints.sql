-- Phase 4 of retiring the App abstraction (EVE-1003): the channel row becomes
-- the exposure, owned directly by an Agent.
--
-- See knowledge/integrations/agent-exposure.md, "Model" and "Invariants that
-- must not move". `app_channels` becomes a view over `agent_endpoints` for one
-- release so App read paths keep working while their callers move.
--
-- Ordering: runs after 134, which guarantees every App has an Agent. That is
-- what lets `agent_endpoints.agent_id` be NOT NULL.

-- Serialize App and channel writers so a channel created during a rolling
-- deployment is visible to the snapshot below instead of being dropped by it.
-- Same reasoning as 134.
LOCK TABLE apps IN SHARE ROW EXCLUSIVE MODE;
LOCK TABLE app_channels IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM apps WHERE agent_id IS NULL) THEN
        RAISE EXCEPTION 'apps.agent_id must be backfilled (migration 134) before endpoints can be re-parented onto agents';
    END IF;
END;
$$;

CREATE TABLE agent_endpoints (
    -- Preserved from app_channels.id on backfill. The channel id is the ingress
    -- URL identity: it appears in registered Slack manifests, published A2A
    -- agent cards, and public chat links. Regenerating it breaks live installs.
    -- New rows keep app_channels' server-side default.
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    -- Transitional. Retained so `app_channels` can stay a view for one release
    -- and so App-scoped reads (org lookups, the schedule cap) keep working.
    -- Dropped with the `apps` table in EVE-1011.
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    -- Preserved verbatim, `appchan_`-prefixed, for the same reason as `id`.
    public_id TEXT NOT NULL,
    channel_type VARCHAR(50) NOT NULL
        CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat')),
    channel_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- When encryption is configured the entire config lives here and
    -- `channel_config` is stored as `{}`. See prepare_channel_config().
    channel_config_encrypted BYTEA,
    durable_schedule_id UUID REFERENCES durable_schedules(id) ON DELETE SET NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    -- Per-endpoint lifecycle. Populated here so the column exists and carries an
    -- honest derived value, but publish still reads App-level status until the
    -- publish phase (EVE-1007) makes this authoritative.
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'live', 'disabled')),
    -- Nullable: falls back to the agent's identity when unset.
    agent_identity_id UUID REFERENCES agent_identities(id),
    agent_version_policy TEXT NOT NULL DEFAULT 'default'
        CHECK (agent_version_policy IN ('default', 'latest', 'pinned')),
    agent_version_id UUID REFERENCES agent_versions(id) ON DELETE SET NULL,
    -- Load-bearing for security, and mandatory on the endpoint. App-channel
    -- ingress runs as Caller::internal(org), whose default principal is the
    -- system principal; sessions adopt this owner instead. `shared_session`
    -- reuse keys on the owner, so collapsing this to the agent would make reuse
    -- never match and ownership unaccountable.
    -- See knowledge/integrations/app-invocation-channels.md, TM-AUTHZ-009, TM-A2A-007.
    owner_principal_id UUID NOT NULL REFERENCES principals(id),
    resolved_owner_user_id UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (public_id),
    CONSTRAINT agent_endpoints_public_id_format CHECK (public_id ~ '^appchan_[0-9a-f]{32}$')
);

-- Backfill from app_channels ⋈ apps, taking agent_id and the lifted exposure
-- policy from the owning App. Ids and public_ids are carried over unchanged.
INSERT INTO agent_endpoints (
    id,
    agent_id,
    app_id,
    public_id,
    channel_type,
    channel_config,
    channel_config_encrypted,
    durable_schedule_id,
    enabled,
    status,
    agent_identity_id,
    agent_version_policy,
    agent_version_id,
    owner_principal_id,
    resolved_owner_user_id,
    created_at,
    updated_at
)
SELECT
    ac.id,
    app.agent_id,
    ac.app_id,
    ac.public_id,
    ac.channel_type,
    ac.channel_config,
    ac.channel_config_encrypted,
    ac.durable_schedule_id,
    ac.enabled,
    -- Derived from the 2-D App.status × channel.enabled matrix this phase is
    -- replacing: an endpoint is only live when the App is published AND the
    -- channel is enabled.
    CASE
        WHEN NOT ac.enabled THEN 'disabled'
        WHEN app.status = 'published' THEN 'live'
        ELSE 'draft'
    END,
    app.agent_identity_id,
    app.agent_version_policy,
    app.agent_version_id,
    app.owner_principal_id,
    app.resolved_owner_user_id,
    ac.created_at,
    ac.updated_at
FROM app_channels AS ac
JOIN apps AS app ON app.id = ac.app_id;

DO $$
DECLARE
    unmigrated BIGINT;
BEGIN
    SELECT COUNT(*)
    INTO unmigrated
    FROM app_channels AS ac
    LEFT JOIN agent_endpoints AS ae ON ae.id = ac.id
    WHERE ae.id IS NULL;

    IF unmigrated > 0 THEN
        RAISE EXCEPTION 'agent_endpoints backfill left % app_channels row(s) without an endpoint', unmigrated;
    END IF;
END;
$$;

CREATE INDEX idx_agent_endpoints_agent_id ON agent_endpoints(agent_id);
CREATE INDEX idx_agent_endpoints_app_id ON agent_endpoints(app_id);
-- Retains the partial-unique semantics 021 gave app_channels.durable_schedule_id.
CREATE UNIQUE INDEX agent_endpoints_durable_schedule_id_idx
    ON agent_endpoints (durable_schedule_id)
    WHERE durable_schedule_id IS NOT NULL;

CREATE TRIGGER update_agent_endpoints_updated_at
    BEFORE UPDATE ON agent_endpoints
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Replace app_channels with a read view exposing exactly its former columns.
-- App read paths keep working unchanged; writers move to agent_endpoints in the
-- same change that adds this migration.
DROP TABLE app_channels;

CREATE VIEW app_channels AS
SELECT
    id,
    app_id,
    public_id,
    channel_type,
    channel_config,
    channel_config_encrypted,
    durable_schedule_id,
    enabled,
    created_at,
    updated_at
FROM agent_endpoints;
