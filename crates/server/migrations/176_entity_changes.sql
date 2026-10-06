-- Entity history: one row per recorded change to an editable entity (agents,
-- harnesses, knowledge, connections, ...), written by `Command::run` after a
-- mutating command succeeds. It answers "who changed this, through what, and
-- why", so a later reader (a person or Platform Chat) can explain a change.
--
-- Append-only. Rows outlive the entity they describe: deleting an agent keeps
-- its history readable to managers of the org. `entity_ref` is the entity's
-- public id as text, so one table serves every kind.
--
-- `reason` is caller text and never authoritative; the actor columns are
-- derived by the server. `revision`, `snapshot` and `snapshot_hash` are filled
-- once snapshots land (see
-- knowledge/execution/change-reasons-and-manager-context.md); until then they
-- stay NULL.
CREATE TABLE entity_changes (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL,
    entity_ref TEXT NOT NULL,
    command TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT NULL,
    changed_fields TEXT[] NOT NULL DEFAULT '{}',
    actor_kind TEXT NOT NULL,
    actor_user_id UUID NULL,
    via_session_id UUID NULL,
    via_agent_id TEXT NULL,
    surface TEXT NOT NULL,
    request_id TEXT NULL,
    idempotency_key TEXT NULL,
    revision BIGINT NULL,
    snapshot JSONB NULL,
    snapshot_hash TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT entity_changes_reason_length CHECK (reason IS NULL OR char_length(reason) BETWEEN 1 AND 1000)
);

CREATE INDEX entity_changes_entity_idx
    ON entity_changes (org_id, entity_kind, entity_ref, created_at DESC, id DESC);
CREATE INDEX entity_changes_org_idx
    ON entity_changes (org_id, created_at DESC, id DESC);
