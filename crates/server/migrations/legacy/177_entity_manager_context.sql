-- Manager context: one markdown document per entity, written by the people
-- (and agents) who manage it, for each other: requirements, rationale,
-- ownership. Kept out of the entity's own row on purpose, so it never flows
-- into anything the entity's runtime reads (prompt, resolved config, exports,
-- previews, events). Reading it needs the entity kind's manage policy.
--
-- `revision` starts at 1 and grows by one per write, so a writer can say which
-- revision it read (`--expected-revision`, `--context-revision`). Clearing
-- keeps the row with empty content, so revisions never repeat. Every write
-- also records a `context_updated` entry in `entity_changes`; the row is
-- removed when its entity is deleted.
--
-- See knowledge/execution/change-reasons-and-manager-context.md.
CREATE TABLE entity_manager_context (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL,
    entity_ref TEXT NOT NULL,
    content TEXT NOT NULL,
    revision BIGINT NOT NULL,
    updated_by_user_id UUID NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, entity_kind, entity_ref),
    CONSTRAINT entity_manager_context_size CHECK (octet_length(content) <= 16384),
    CONSTRAINT entity_manager_context_revision CHECK (revision >= 1)
);
