-- Durable physical-incarnation state for logical sandboxes.
--
-- `sandboxes` already owns logical identity and checkpoint lineage. This adds
-- queryable provider-resource state beneath it so recovery no longer depends
-- on the encrypted `session_sandbox` secret. Existing secret records are
-- adopted lazily on first access.

CREATE TABLE sandbox_instances (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    sandbox_id        UUID        NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    generation        BIGINT      NOT NULL CHECK (generation > 0),
    external_id       TEXT        NOT NULL,
    display_name      TEXT,
    workspace_path    TEXT,
    provider_state    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    metadata          JSONB       NOT NULL DEFAULT '{}'::jsonb,
    status            TEXT        NOT NULL
                      CHECK (status IN ('running', 'paused', 'lost')),
    init_completed_at TIMESTAMPTZ,
    last_init_error   TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at        TIMESTAMPTZ,
    CONSTRAINT sandbox_instances_generation_uq UNIQUE (sandbox_id, generation)
);

ALTER TABLE sandboxes
    ADD COLUMN current_instance_id UUID;

ALTER TABLE sandboxes
    ADD CONSTRAINT sandboxes_current_instance_fk
    FOREIGN KEY (current_instance_id) REFERENCES sandbox_instances(id)
    ON DELETE SET NULL;

CREATE INDEX idx_sandbox_instances_external
    ON sandbox_instances (external_id)
    WHERE retired_at IS NULL;

COMMENT ON TABLE sandbox_instances IS
    'Physical provider incarnations of a durable logical sandbox. Replacements '
    'advance sandboxes.generation and retire the previous row.';

COMMENT ON COLUMN sandbox_instances.provider_state IS
    'Opaque non-secret provider state. Credentials must never be persisted here.';
