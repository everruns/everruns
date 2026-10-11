-- The shared channel host's store (knowledge/integrations/channels.md,
-- "Server on the host").
--
-- 1. channel_bindings: which session a channel conversation (a Slack thread,
--    an AG-UI thread, a webhook's shared session) continues in. The primary
--    key makes binding first-writer-wins: two first messages of one
--    conversation can no longer start two sessions.
-- 2. channel_seen: platform delivery keys already accepted, so a retried
--    delivery starts nothing. Kept a day; platforms retry within minutes.
-- 3. channel_pending_deliveries: replies still owed to a platform, leased to
--    one server instance at a time so a restart, or several instances,
--    deliver each reply once.
--
-- Every row is keyed by the channel row's id, never its name, so keys cannot
-- cross tenants, and goes with its channel and session. Each channel's move
-- onto the host backfills its existing tag bindings in its own migration.

CREATE TABLE channel_bindings (
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    binding_key TEXT NOT NULL,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel_id, binding_key)
);
CREATE INDEX idx_channel_bindings_session ON channel_bindings(session_id);

CREATE TABLE channel_seen (
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel_id, dedup_key)
);
CREATE INDEX idx_channel_seen_age ON channel_seen(channel_id, seen_at);

CREATE TABLE channel_pending_deliveries (
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    input_message_id TEXT NOT NULL,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    target JSONB NOT NULL,
    after_sequence BIGINT NOT NULL DEFAULT 0,
    holder TEXT NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, input_message_id)
);
CREATE INDEX idx_channel_pending_deliveries_lease ON channel_pending_deliveries(lease_until);
