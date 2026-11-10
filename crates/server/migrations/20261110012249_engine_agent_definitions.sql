-- Platform agents as Engine definitions (data plane v2 step 4).
--
-- The Engine never sees a platform id. The control plane mints an opaque key
-- for each (organization, harness, agent) a session can run on, compiles that
-- combination into an id-free declaration (everruns::hosting::RuntimeAgent)
-- and publishes it as numbered revisions under the key. Only this table maps a
-- key back to platform records.
CREATE TABLE engine_agent_keys (
    agent_key TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    harness_id UUID NOT NULL,
    -- NULL for a harness-only session.
    agent_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX engine_agent_keys_target
    ON engine_agent_keys (org_id, harness_id, COALESCE(agent_id, '00000000-0000-0000-0000-000000000000'::uuid));
CREATE INDEX engine_agent_keys_agent ON engine_agent_keys (org_id, agent_id) WHERE agent_id IS NOT NULL;

-- Every published revision, kept forever: a retained turn rebuilds exactly
-- what it started with.
CREATE TABLE engine_agent_definitions (
    agent_key TEXT NOT NULL REFERENCES engine_agent_keys(agent_key) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    runtime_agent JSONB NOT NULL,
    -- sha256 of the declaration; an edit that compiles to the same digest
    -- publishes nothing.
    digest TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, revision)
);

-- Each key's current revision: what a session moves to between turns.
CREATE TABLE engine_agent_current (
    agent_key TEXT PRIMARY KEY REFERENCES engine_agent_keys(agent_key) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    digest TEXT NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Edits that may change what keys compile to. Written in the edit's own
-- transaction (an edit is never lost between the two), drained by the
-- publisher. NULL agent and harness ids mean every key of the organization
-- (an organization default changed).
CREATE TABLE engine_agent_publish_queue (
    id BIGSERIAL PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    agent_id UUID,
    harness_id UUID,
    reason TEXT NOT NULL,
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A session's Engine binding, set when it first runs on the Engine. The key
-- and revision say which definition its next turn hydrates; the claim moves
-- the revision to the key's current one between turns. NULL keys run on the
-- worker's own host as before.
ALTER TABLE session_actors
    ADD COLUMN engine_agent_key TEXT REFERENCES engine_agent_keys(agent_key),
    ADD COLUMN engine_revision TEXT,
    ADD COLUMN engine_runtime INTEGER,
    -- Why the Engine suspended the session (it is not claimed again until a
    -- person or operator resumes it).
    ADD COLUMN engine_suspended_reason TEXT,
    -- When a waiting session becomes runnable again (Engine timers).
    ADD COLUMN engine_wake_at TIMESTAMPTZ,
    ADD CONSTRAINT session_actors_engine_binding
        CHECK ((engine_agent_key IS NULL) = (engine_revision IS NULL));
CREATE INDEX session_actors_engine_wake
    ON session_actors (engine_wake_at)
    WHERE engine_wake_at IS NOT NULL AND engine_suspended_reason IS NULL;

-- Engine session records (turn assignments, outcomes, receipts), written
-- under the actor's lease fence like every journal append.
CREATE TABLE engine_session_records (
    session_id UUID NOT NULL,
    record_key TEXT NOT NULL,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, record_key)
);
