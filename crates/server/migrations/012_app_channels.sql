-- Migration 012: Extract app channels into a separate table
-- Supports multiple channels per app (multi-channel messaging).
-- Data migration: copy existing channel_type/channel_config from apps into app_channels.

-- ============================================
-- App Channels table
-- ============================================

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
    UNIQUE (public_id)
);

CREATE INDEX idx_app_channels_app_id ON app_channels(app_id);
CREATE INDEX idx_app_channels_public_id ON app_channels(public_id);

CREATE TRIGGER update_app_channels_updated_at
    BEFORE UPDATE ON app_channels
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ============================================
-- Data migration: copy existing app channels
-- ============================================
-- Generate a new UUIDv7 public_id for each existing app's channel.
-- Use the app's public_id to derive a deterministic channel public_id.

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
