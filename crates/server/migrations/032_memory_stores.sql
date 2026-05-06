-- Org-scoped persistent memory stores (EVE-422).
-- See specs/memory.md for full design intent.
--
-- A MemoryStore is an org-scoped named container for Memory entries that
-- agents create and recall via the `memory` capability (remember/recall/forget
-- tools). Each org has at most one default store, automatically created on
-- first use. Capability config can target a specific store ID.

CREATE TABLE memory_stores (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT memory_stores_public_id_format
        CHECK (public_id ~ '^mst_[0-9a-f]{32}$')
);

CREATE INDEX idx_memory_stores_org_id ON memory_stores(org_id);
CREATE UNIQUE INDEX idx_memory_stores_org_public_id ON memory_stores(org_id, public_id);
CREATE UNIQUE INDEX idx_memory_stores_org_lower_name ON memory_stores(org_id, LOWER(name));
-- Only one default store per org.
CREATE UNIQUE INDEX idx_memory_stores_org_default ON memory_stores(org_id) WHERE is_default;

CREATE TRIGGER update_memory_stores_updated_at BEFORE UPDATE ON memory_stores
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE memory_stores IS 'Org-scoped containers for agent memories. See specs/memory.md.';

CREATE TABLE memories (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    public_id TEXT NOT NULL,
    store_id UUID NOT NULL REFERENCES memory_stores(id) ON DELETE CASCADE,
    -- org_id duplicated from store for fast org-scoped queries and isolation checks.
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    content TEXT NOT NULL,
    -- Multicontent parts (text + image variants) as JSON array. Empty array if
    -- the memory only carries the plain `content` field.
    content_parts JSONB NOT NULL DEFAULT '[]'::jsonb,
    kind VARCHAR(32) NOT NULL DEFAULT 'fact'
        CHECK (kind IN ('fact', 'preference', 'correction', 'procedure', 'context')),
    importance SMALLINT NOT NULL DEFAULT 5
        CHECK (importance BETWEEN 1 AND 10),
    tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT memories_public_id_format
        CHECK (public_id ~ '^mem_[0-9a-f]{32}$'),
    CONSTRAINT memories_content_length CHECK (char_length(content) <= 2000)
);

CREATE INDEX idx_memories_store_active ON memories(store_id) WHERE active;
CREATE INDEX idx_memories_org_id ON memories(org_id);
CREATE UNIQUE INDEX idx_memories_public_id ON memories(public_id);
CREATE INDEX idx_memories_store_kind_active ON memories(store_id, kind) WHERE active;
CREATE INDEX idx_memories_tags_gin ON memories USING GIN (tags);

CREATE TRIGGER update_memories_updated_at BEFORE UPDATE ON memories
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE memories IS 'Memory entries inside a memory_store. Soft-deleted via active=false.';
