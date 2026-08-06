-- Allow `public_chat` app channels.
-- An isolated, public-facing chat web app bound to a single App's agent.
-- Runs after 083 (api_endpoint), so the constraint list must retain
-- `api_endpoint` as well. See knowledge/integrations/public-chat.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;

ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat'));

ALTER TABLE app_channels
    DROP CONSTRAINT IF EXISTS app_channels_channel_type_check;

ALTER TABLE app_channels
    ADD CONSTRAINT app_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', 'public_chat'));
