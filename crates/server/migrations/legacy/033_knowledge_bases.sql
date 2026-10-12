-- Knowledge Bases (EVE-423)
-- See specs/knowledge-bases.md for full design intent.
--
-- A Knowledge Base is an org-scoped, named collection of curated text entries
-- (table docs, business rules, validated SQL templates, runbooks, etc.) that
-- agents search through a `search_knowledge` tool. Knowledge Bases sit
-- alongside Memory (specs/memory.md, agent-written facts) and Workspace
-- Volumes (specs/volumes.md, file trees mounted into /workspace).
--
-- This migration adds the foundational schema:
--   - knowledge_bases:    the KB entity (lifecycle: active/archived/deleted)
--   - knowledge_entries:  individual curated entries (title + markdown body)
--
-- The agent-facing `search_knowledge` tool, capability config UI, and KB
-- management UI ship in follow-up vertical slices on top of this foundation.

-- ============================================
-- Knowledge Bases (org-scoped curated collections)
-- ============================================

CREATE TABLE knowledge_bases (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- Owner principal (creator). Resolution to a concrete user happens at the
    -- domain layer; matches the pattern used by volumes and other entities.
    owner_principal_id TEXT,
    resolved_owner_user_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,

    -- Format validation: kb_ prefix + 32 lowercase hex chars
    CONSTRAINT knowledge_bases_public_id_format
        CHECK (public_id ~ '^kb_[0-9a-f]{32}$')
);

CREATE INDEX idx_knowledge_bases_org_id ON knowledge_bases(org_id);
CREATE INDEX idx_knowledge_bases_status ON knowledge_bases(status);
-- Unique per org (same public_id allowed across orgs)
CREATE UNIQUE INDEX idx_knowledge_bases_org_public_id ON knowledge_bases(org_id, public_id);
-- Case-insensitive unique name per org while not deleted (tombstones may
-- share names with new rows).
CREATE UNIQUE INDEX idx_knowledge_bases_org_name_active
    ON knowledge_bases(org_id, LOWER(name))
    WHERE status != 'deleted';

CREATE TRIGGER update_knowledge_bases_updated_at BEFORE UPDATE ON knowledge_bases
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE knowledge_bases IS 'Knowledge Bases — org-scoped curated entry collections. See specs/knowledge-bases.md.';

-- ============================================
-- Knowledge Entries (curated docs inside a KB)
-- ============================================

CREATE TABLE knowledge_entries (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    kb_id UUID NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
    public_id TEXT NOT NULL,

    title VARCHAR(512) NOT NULL,
    -- Markdown body. Hard-capped at 64 KiB to bound storage, search-index
    -- size, and tool response payloads.
    body TEXT NOT NULL,
    kind VARCHAR(32) NOT NULL DEFAULT 'note'
        CHECK (kind IN ('note', 'table', 'business', 'query', 'runbook')),
    tags TEXT[] NOT NULL DEFAULT '{}',

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT knowledge_entries_public_id_format
        CHECK (public_id ~ '^kbe_[0-9a-f]{32}$'),
    CONSTRAINT knowledge_entries_body_size
        CHECK (octet_length(body) <= 65536),
    CONSTRAINT knowledge_entries_tags_count
        CHECK (cardinality(tags) <= 16)
);

CREATE UNIQUE INDEX idx_knowledge_entries_kb_public_id
    ON knowledge_entries(kb_id, public_id);
CREATE INDEX idx_knowledge_entries_kb_id ON knowledge_entries(kb_id);
CREATE INDEX idx_knowledge_entries_kb_kind ON knowledge_entries(kb_id, kind);
-- Full-text index for `search_knowledge`; the agent-facing tool queries
-- this in the follow-up PR. Building it here avoids a follow-up migration
-- and makes early CRUD smoke tests representative.
CREATE INDEX idx_knowledge_entries_fts
    ON knowledge_entries
    USING GIN (to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, '')));

CREATE TRIGGER update_knowledge_entries_updated_at BEFORE UPDATE ON knowledge_entries
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE knowledge_entries IS 'Curated entries inside a Knowledge Base. Title + markdown body, kind-tagged.';
