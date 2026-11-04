-- Add Work Chat while preserving the retired-type tightening policy. Only
-- deployments with unconverted encrypted channels still admit api_endpoint.
DO $$
BEGIN
    ALTER TABLE agent_channels DROP CONSTRAINT agent_channels_channel_type_check;
    IF EXISTS (SELECT 1 FROM agent_channels WHERE channel_type = 'api_endpoint') THEN
        ALTER TABLE agent_channels ADD CONSTRAINT agent_channels_channel_type_check
            CHECK (channel_type IN ('slack','ag_ui','schedule','webhook','a2a','fcp',
                'public_chat','voice','api','poppy','api_endpoint','work_chat'));
    ELSE
        ALTER TABLE agent_channels ADD CONSTRAINT agent_channels_channel_type_check
            CHECK (channel_type IN ('slack','ag_ui','schedule','webhook','a2a','fcp',
                'public_chat','voice','api','poppy','work_chat'));
    END IF;
END $$;

-- The optional standalone browser host stores only encrypted provider tokens
-- and hashed opaque credentials. Single-use login state survives restarts.
CREATE TABLE hosted_chat_credentials (
    token_hash TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('flow','handoff','session','limit')),
    payload TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX hosted_chat_credentials_expiry ON hosted_chat_credentials (expires_at);
