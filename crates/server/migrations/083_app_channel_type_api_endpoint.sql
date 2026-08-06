-- Allow the `api_endpoint` channel type on apps and app_channels.
-- See knowledge/integrations/app-api-keys.md.

ALTER TABLE apps
    DROP CONSTRAINT IF EXISTS apps_channel_type_check;
ALTER TABLE apps
    ADD CONSTRAINT apps_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint'));

ALTER TABLE app_channels
    DROP CONSTRAINT IF EXISTS app_channels_channel_type_check;
ALTER TABLE app_channels
    ADD CONSTRAINT app_channels_channel_type_check
    CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint'));
