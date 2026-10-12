-- Personal Agent Protocol ("Poppy", draft 0.1) channels: a company's front
-- door for personal agents (knowledge/integrations/poppy-channel.md).
--
-- 1. channel_type 'poppy' on apps and agent_channels.
-- 2. poppy_sessions: one row per Poppy Session (spec 4): one user, one
--    personal agent, one channel. Session tokens name the row, so ending it
--    stops every token issued for it. `account` is the company account the
--    session is signed in to, once sign-in exists.
-- 3. poppy_conversations: one row per conversation (spec 7), backed by one
--    Everruns session. Owned by (client_id, user_id) until it uses an account.
-- 4. poppy_messages: personal-agent message ids already accepted, unique per
--    user (spec 7.3), with a hash of the content so a retry with the same id
--    and different content is refused.
-- Signing keys reuse pact_signing_keys, which is keyed by channel.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice', 'api', 'poppy'));

ALTER TABLE agent_channels
    DROP CONSTRAINT IF EXISTS agent_channels_channel_type_check;

ALTER TABLE agent_channels
    ADD CONSTRAINT agent_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat', 'voice', 'api', 'poppy'));

CREATE TABLE poppy_sessions (
    id TEXT PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    account TEXT,
    scope TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    renewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at TIMESTAMPTZ
);
CREATE INDEX poppy_sessions_user ON poppy_sessions(channel_id, client_id, user_id);

CREATE TABLE poppy_conversations (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    account TEXT,
    context JSONB NOT NULL DEFAULT '{}'::jsonb,
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX poppy_conversations_owner ON poppy_conversations(channel_id, client_id, user_id);

CREATE TABLE poppy_messages (
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    content_hash BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel_id, client_id, user_id, message_id)
);
