-- v0.8.14: AG-UI app channel
-- Squashed from: 014_ag_ui_app_channel.sql

-- ============================================
-- 014_ag_ui_app_channel: Allow 'ag_ui' as an app channel_type
-- Extends the existing CHECK constraint on apps / app_channels so AG-UI apps
-- can register alongside Slack.
-- ============================================

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui'));

ALTER TABLE app_channels
    DROP CONSTRAINT IF EXISTS app_channels_channel_type_check;

ALTER TABLE app_channels
    ADD CONSTRAINT app_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui'));
