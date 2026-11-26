-- email_inboxes
-- Provider account credentials stay in organization connections. Inbox identity
-- and receipt deduplication survive key rotation and channel reconfiguration.
CREATE TABLE email_inboxes (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES virtual_user_connections(id) ON DELETE RESTRICT,
    provider TEXT NOT NULL CHECK (provider = 'agentmail'),
    provider_inbox_id TEXT NOT NULL,
    address TEXT NOT NULL,
    name TEXT NOT NULL,
    signing_secret_encrypted BYTEA NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, provider, provider_inbox_id),
    UNIQUE (org_id, id)
);

DO $$
BEGIN
    ALTER TABLE agent_channels DROP CONSTRAINT agent_channels_channel_type_check;
    IF EXISTS (SELECT 1 FROM agent_channels WHERE channel_type = 'api_endpoint') THEN
        ALTER TABLE agent_channels ADD CONSTRAINT agent_channels_channel_type_check
            CHECK (channel_type IN ('slack','ag_ui','schedule','webhook','a2a','fcp',
                'public_chat','voice','api','poppy','api_endpoint','work_chat','email'));
    ELSE
        ALTER TABLE agent_channels ADD CONSTRAINT agent_channels_channel_type_check
            CHECK (channel_type IN ('slack','ag_ui','schedule','webhook','a2a','fcp',
                'public_chat','voice','api','poppy','work_chat','email'));
    END IF;
END $$;
CREATE TABLE email_channel_bindings (
    org_id BIGINT NOT NULL,
    inbox_id UUID NOT NULL,
    channel_id UUID NOT NULL REFERENCES agent_channels(id) ON DELETE CASCADE,
    PRIMARY KEY (org_id,inbox_id),
    UNIQUE(channel_id),
    FOREIGN KEY (org_id,inbox_id) REFERENCES email_inboxes(org_id,id) ON DELETE CASCADE
);

CREATE TABLE email_intake (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL,
    inbox_id UUID NOT NULL,
    event_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    subject TEXT NOT NULL,
    preview TEXT NOT NULL,
    message JSONB NOT NULL,
    stored_bytes INTEGER NOT NULL CHECK (stored_bytes BETWEEN 0 AND 1048576),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','triaging','ready','running','completed','ignored','held','deferred','failed')),
    reason TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    lease_id UUID,
    lease_until TIMESTAMPTZ,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (org_id, inbox_id) REFERENCES email_inboxes(org_id, id) ON DELETE CASCADE,
    UNIQUE (inbox_id, event_id),
    UNIQUE (inbox_id, message_id)
);
CREATE INDEX email_intake_pending ON email_intake (org_id, inbox_id, available_at, received_at) WHERE state IN ('pending','deferred','triaging');
CREATE INDEX email_intake_thread ON email_intake (inbox_id, thread_id, received_at);

CREATE TABLE email_outbox (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL,
    inbox_id UUID NOT NULL,
    intake_id UUID NOT NULL REFERENCES email_intake(id) ON DELETE RESTRICT,
    session_id UUID NOT NULL,
    output_key TEXT NOT NULL,
    body JSONB NOT NULL,
    state TEXT NOT NULL DEFAULT 'held' CHECK (state IN ('held','sending','sent','unknown','failed')),
    provider_message_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (org_id,inbox_id) REFERENCES email_inboxes(org_id,id) ON DELETE RESTRICT,
    UNIQUE (org_id,inbox_id,output_key)
);

CREATE TABLE email_templates (
    id UUID NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    version INTEGER NOT NULL CHECK (version > 0),
    name TEXT NOT NULL,
    subject TEXT NOT NULL,
    html TEXT,
    text_body TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id, version),
    CHECK (html IS NOT NULL OR text_body IS NOT NULL)
);

-- Scope the inbox binding to the channel's owning organization as well.
CREATE OR REPLACE FUNCTION check_email_channel_binding() RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM agent_channels c JOIN agents a ON a.id=c.agent_id WHERE c.id=NEW.channel_id
        AND a.org_id=NEW.org_id AND c.channel_type='email') THEN
        RAISE EXCEPTION 'Invalid email channel binding';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER email_binding_scope BEFORE INSERT OR UPDATE ON email_channel_bindings
FOR EACH ROW EXECUTE FUNCTION check_email_channel_binding();

ALTER TABLE email_intake ADD COLUMN session_id UUID;
ALTER TABLE email_intake ADD COLUMN approved BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE email_intake ADD COLUMN dispatch_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE email_inboxes ADD COLUMN last_polled_at TIMESTAMPTZ;
ALTER TABLE email_inboxes ADD COLUMN last_scheduled_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
ALTER TABLE email_inboxes ADD COLUMN last_triaged_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
ALTER TABLE email_outbox ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE sessions DROP CONSTRAINT sessions_source_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_source_check CHECK (source IN
('chat','playground','api','slack','email','ag_ui','fcp','schedule','webhook','a2a','eval','subagent','room','unknown'));

CREATE TABLE email_rate_limits (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (org_id,kind,window_start)
);

ALTER TABLE email_outbox ADD COLUMN gate_state TEXT NOT NULL DEFAULT 'pending' CHECK (gate_state IN ('pending','passed','review'));

ALTER TABLE email_inboxes ADD COLUMN poll_after TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_inboxes ADD COLUMN poll_started_at TIMESTAMPTZ;
ALTER TABLE email_inboxes ADD COLUMN poll_page TEXT;

CREATE UNIQUE INDEX email_outbox_one_reply ON email_outbox(intake_id);

ALTER TABLE email_outbox ADD COLUMN authorized BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE email_outbox ADD COLUMN approved_by UUID;
ALTER TABLE email_outbox ADD COLUMN available_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_intake ADD COLUMN approved_by UUID;

ALTER TABLE email_intake ADD COLUMN archived_at TIMESTAMPTZ;
CREATE INDEX email_intake_retention ON email_intake(updated_at) WHERE archived_at IS NULL AND state IN ('completed','ignored','failed');
