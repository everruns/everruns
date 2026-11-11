-- The control plane's side of /dp/v1 (data plane v2 step 7,
-- knowledge/execution/data-plane-v2/cp-dp-protocol.md). An org whose row is
-- in dp_orgs runs on a data plane (v2); every other org runs on the
-- platform's own workers (v1), unchanged.

-- Data planes this control plane serves. The link's token and wrapping key
-- come from the server's environment (one data plane per deployment until
-- step 10 moves them to mTLS and KMS).
CREATE TABLE data_planes (
    data_plane_id TEXT PRIMARY KEY,
    -- A gateway's base URL serving /dp/v1 config.
    url TEXT NOT NULL,
    -- A node's base URL serving /dp/v1 org admin and feed replay.
    admin_url TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The control plane's epoch: bumped by an operator after a database
-- restore, so the data planes take a full snapshot and refuse stale updates.
CREATE TABLE dp_control_epoch (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    epoch BIGINT NOT NULL
);
INSERT INTO dp_control_epoch (singleton, epoch) VALUES (true, 1);

-- An org on a data plane, its config position and its data key.
CREATE TABLE dp_orgs (
    org_id BIGINT PRIMARY KEY REFERENCES organizations(org_id) ON DELETE CASCADE,
    -- Row id for secrets key rotation, which walks rows by a UUID.
    id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    data_plane_id TEXT NOT NULL REFERENCES data_planes(data_plane_id),
    -- The latest config version assigned; items carry the version that
    -- last changed them.
    version BIGINT NOT NULL DEFAULT 0,
    delivered_epoch BIGINT NOT NULL DEFAULT 0,
    delivered_version BIGINT NOT NULL DEFAULT 0,
    -- The org's data key, encrypted with the server's secrets key (a v2
    -- org needs one). Sent to the data plane wrapped with its key only.
    data_key_id TEXT NOT NULL,
    data_key_encrypted BYTEA NOT NULL,
    -- OrgEnable acknowledged by the data plane.
    enabled BOOLEAN NOT NULL DEFAULT false,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'suspended')),
    -- Usage cost the feed has journaled, against the grant ledger.
    feed_cost_micros BIGINT NOT NULL DEFAULT 0,
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- What each v2 org's config holds. The outbox is this table: an update
-- carries every item changed since the version the data plane applied, a
-- full snapshot every live item.
CREATE TABLE dp_config_items (
    org_id BIGINT NOT NULL REFERENCES dp_orgs(org_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'secret', 'connection', 'policy')),
    item_id TEXT NOT NULL,
    revision BIGINT NOT NULL,
    -- The change as sent; NULL once removed (sent as a removal).
    body JSONB,
    version BIGINT NOT NULL,
    PRIMARY KEY (org_id, kind, item_id)
);
CREATE INDEX dp_config_items_version ON dp_config_items (org_id, version);

-- How far each source's feed is stored; a record at or before it is a
-- repeat.
CREATE TABLE dp_feed_sources (
    source TEXT NOT NULL,
    incarnation BIGINT NOT NULL,
    stored_index BIGINT NOT NULL,
    stored_ordinal BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (source, incarnation)
);

-- v2 sessions as the feed summarizes them, for lists and search.
CREATE TABLE dp_session_summaries (
    session_id TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    data_plane_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    session_version BIGINT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    turns BIGINT NOT NULL DEFAULT 0,
    cost_micros BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX dp_session_summaries_org ON dp_session_summaries (org_id, last_activity_at DESC);

-- Spend allowances: money reserved against the org's usage policies when
-- granted, released only by a closed settlement.
CREATE TABLE dp_grants (
    grant_id TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    cell TEXT NOT NULL,
    granted_micros BIGINT NOT NULL,
    used_micros BIGINT NOT NULL DEFAULT 0,
    reserved_micros BIGINT NOT NULL DEFAULT 0,
    closed BOOLEAN NOT NULL DEFAULT false,
    spend_until TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ
);
CREATE INDEX dp_grants_open ON dp_grants (org_id) WHERE NOT closed;

-- Each allowance request's answer: a retry gets the same one.
CREATE TABLE dp_allowance_requests (
    request_id TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    answer JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Org events the data plane reported (suspended, budget exhausted, ...).
CREATE TABLE dp_org_events (
    id BIGSERIAL PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL
);
CREATE INDEX dp_org_events_org ON dp_org_events (org_id, at DESC);

-- Keys that sign org tokens (EdDSA). Published at /.well-known/jwks.json;
-- a retired key stays published until the tokens it signed have expired.
CREATE TABLE dp_signing_keys (
    kid TEXT PRIMARY KEY,
    -- Row id for secrets key rotation, which walks rows by a UUID.
    id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    -- PKCS#8, encrypted with the server's secrets key.
    private_key_encrypted BYTEA NOT NULL,
    public_jwk JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ
);
