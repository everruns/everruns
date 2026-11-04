-- Account edits have a user boundary, independent of organization membership.
-- Reuse the history shape, but never mix private profile snapshots into an
-- organization's history query. Account deletion erases the entire record.
CREATE TABLE account_entity_changes (
    id uuid PRIMARY KEY,
    org_id bigint NOT NULL CHECK (org_id = 0),
    entity_kind text NOT NULL CHECK (entity_kind = 'user_profile'),
    entity_ref text NOT NULL,
    owner_user_id uuid GENERATED ALWAYS AS (entity_ref::uuid) STORED
        REFERENCES users(id) ON DELETE CASCADE,
    command text NOT NULL,
    action text NOT NULL,
    reason text CHECK (reason IS NULL OR char_length(reason) BETWEEN 1 AND 1000),
    changed_fields text[] NOT NULL DEFAULT '{}',
    actor_kind text NOT NULL,
    actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    via_session_id uuid REFERENCES sessions(id) ON DELETE SET NULL,
    via_agent_id text,
    surface text NOT NULL,
    request_id text,
    idempotency_key text,
    revision bigint,
    snapshot jsonb,
    snapshot_hash text,
    restored_from_revision bigint,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX account_entity_changes_revision
    ON account_entity_changes (entity_kind, entity_ref, revision)
    WHERE revision IS NOT NULL;
CREATE INDEX account_entity_changes_owner_time
    ON account_entity_changes (owner_user_id, created_at DESC, id DESC);
