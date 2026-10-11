-- First-class logical sandbox and sandbox-checkpoint records (EVE-870).
--
-- Until now the authoritative pointer to a recoverable workspace revision was
-- `provider_state.recovery.head_revision` inside the encrypted `session_sandbox`
-- secret (crates/platform/src/session_sandbox.rs). That pointer is written by
-- its own autocommit statement, separately from the durable tool result and the
-- `tool.completed` event, so a control-plane crash between those writes can
-- leave the workspace on a revision no committed tool result corresponds to.
--
-- These tables move sandbox identity and checkpoint history into product state
-- the Act path can reason about transactionally, per
-- `knowledge/harnesses/sandbox-abstraction.md`. This migration is additive: the
-- secret binding remains authoritative for recovery until the reconciliation
-- slice lands, and checkpoints recorded here are observational until then.
--
-- Note: sandbox checkpoints and `session_compaction_checkpoints` (migration 109)
-- are unrelated records. The latter checkpoints conversation state; these
-- checkpoint filesystem state.

CREATE TABLE sandboxes (
    id                UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id            BIGINT       NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    -- Exactly one owner today. Workspace-scoped sandboxes get their own column
    -- when explicit shared scope arrives; until then a session owns its sandbox.
    session_id        UUID         NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    provider          TEXT         NOT NULL,
    -- Incarnation/fencing counter. Every replacement of the physical sandbox
    -- increments it, so a late response from a lost incarnation cannot advance
    -- logical state: writes carry the generation they were issued against.
    generation        BIGINT       NOT NULL DEFAULT 1 CHECK (generation > 0),
    -- The last checkpoint committed as authoritative. NULL until the first
    -- checkpoint is attached. Set by a deferred FK below.
    current_checkpoint_id UUID,
    created_at        TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ  NOT NULL DEFAULT now(),
    -- One logical sandbox per session/provider pair.
    CONSTRAINT sandboxes_session_provider_uq UNIQUE (session_id, provider)
);

COMMENT ON TABLE sandboxes IS
    'Logical sandbox resource (EVE-870). Authoritative product state for a '
    'session workspace; the physical provider resource is a subordinate '
    'incarnation identified by `generation`.';

COMMENT ON COLUMN sandboxes.generation IS
    'Fencing counter. Incremented on every physical replacement so stale '
    'responses from a lost incarnation cannot advance logical state.';

CREATE TABLE sandbox_checkpoints (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    sandbox_id          UUID        NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    -- The incarnation this checkpoint was taken from. A checkpoint from an
    -- older generation can still be restored, but cannot advance the pointer.
    generation          BIGINT      NOT NULL CHECK (generation > 0),
    -- The mutating tool call this checkpoint was taken for. Reconciliation
    -- compares this against `durable_tool_results` to decide whether the
    -- checkpoint belongs to a committed turn.
    source_turn_id      TEXT,
    source_tool_call_id TEXT,
    kind                TEXT        NOT NULL
                        CHECK (kind IN ('provider_native', 'portable_workspace')),
    -- Provider-native snapshot id, or the provider-side revision directory for
    -- a portable workspace archive.
    provider_ref        TEXT,
    -- Provider-independent revision identifier (e.g. Daytona's `rev-<nanos>`).
    workspace_revision  TEXT        NOT NULL,
    -- NULL means uploaded but never committed as authoritative: safe to collect.
    -- Non-NULL means some committed turn may depend on it.
    attached_at         TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT sandbox_checkpoints_revision_uq UNIQUE (sandbox_id, workspace_revision)
);

COMMENT ON TABLE sandbox_checkpoints IS
    'Recoverable workspace revisions (EVE-870). A row is created when the '
    'archive upload succeeds; `attached_at` is set only once the revision is '
    'committed as authoritative.';

COMMENT ON COLUMN sandbox_checkpoints.attached_at IS
    'NULL for uploaded-but-uncommitted checkpoints, which garbage collection '
    'may remove. Never NULL for a revision any committed turn depends on.';

-- The authoritative pointer. Restricted rather than cascading: dropping the
-- checkpoint a sandbox currently points at must fail loudly, so garbage
-- collection can never delete an authoritative revision.
ALTER TABLE sandboxes
    ADD CONSTRAINT sandboxes_current_checkpoint_fk
    FOREIGN KEY (current_checkpoint_id) REFERENCES sandbox_checkpoints(id)
    ON DELETE RESTRICT;

-- Recovery reads the newest checkpoint for a sandbox.
CREATE INDEX idx_sandbox_checkpoints_sandbox_created
    ON sandbox_checkpoints (sandbox_id, created_at DESC);

-- Garbage collection scans only unattached uploads.
CREATE INDEX idx_sandbox_checkpoints_unattached
    ON sandbox_checkpoints (sandbox_id, created_at)
    WHERE attached_at IS NULL;

-- Reconciliation looks a checkpoint up by the tool call that produced it.
CREATE INDEX idx_sandbox_checkpoints_source_tool_call
    ON sandbox_checkpoints (source_turn_id, source_tool_call_id)
    WHERE source_tool_call_id IS NOT NULL;
