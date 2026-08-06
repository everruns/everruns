-- Knowledge Indexes
-- See knowledge/runtime-resources/knowledge-indexes.md for full design intent.
--
-- A Knowledge Index is an org-scoped, named collection that connects an
-- external knowledge source (a GitHub repository in v1), syncs its documents,
-- chunks and embeds them, and exposes semantic + hybrid search with citations.
-- It is the search-optimized, source-backed sibling of curated Knowledge Bases
-- (knowledge/runtime-resources/knowledge-bases.md) and reuses the source-sync pattern from
-- source-backed Memory (knowledge/runtime-resources/memory.md).
--
-- Embedding VECTORS are NOT stored here: Postgres is the management source of
-- truth, while vectors + BM25 text live in an external vector store
-- (Turbopuffer reference backend), one namespace per index, org-prefixed for
-- multitenant isolation. See knowledge/runtime-resources/knowledge-indexes.md#vector-store.
--
-- This migration adds the foundational schema:
--   - knowledge_indexes:           the index entity (lifecycle + sync_status)
--   - knowledge_index_documents:   one ingested source document
--   - knowledge_index_chunks:      the citable retrieval unit (text + location)
--
-- The CRUD API, Syncout worker, `search_index` tool, and management UI ship in
-- follow-up vertical slices on top of this foundation.

-- ============================================
-- Knowledge Indexes (org-scoped source-backed collections)
-- ============================================

CREATE TABLE knowledge_indexes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,

    -- External source coordinates. source_config holds NON-SECRET coordinates
    -- only; credentials are resolved on demand at sync time.
    source_type VARCHAR(50) NOT NULL DEFAULT 'github'
        CHECK (source_type IN ('github', 'git')),
    source_config JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- Required embedding model. An index is inherently embedded; the referenced
    -- model's driver must declare ServiceKind::Embeddings (validated at the
    -- domain layer).
    embedding_model_id UUID NOT NULL REFERENCES models(id),
    -- Embedding dimension, recorded on first successful sync.
    vector_dim INT,
    -- External vector-store namespace, assigned at creation and never reused.
    -- Naming: org_{org_id}__{public_id}. See knowledge/runtime-resources/knowledge-indexes.md.
    vector_namespace TEXT,

    -- Owner principal (creator). Resolution to a concrete user happens at the
    -- domain layer; matches the pattern used by knowledge_bases / memories.
    owner_principal_id TEXT,
    resolved_owner_user_id UUID,

    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted')),
    -- Syncout pipeline state, orthogonal to lifecycle status.
    sync_status VARCHAR(50) NOT NULL DEFAULT 'idle'
        CHECK (sync_status IN ('idle', 'pending', 'syncing', 'synced', 'failed')),
    last_synced_at TIMESTAMPTZ,
    last_sync_error TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,

    CONSTRAINT knowledge_indexes_public_id_format
        CHECK (public_id ~ '^kidx_[0-9a-f]{32}$')
);

CREATE INDEX idx_knowledge_indexes_org_id ON knowledge_indexes(org_id);
CREATE INDEX idx_knowledge_indexes_status ON knowledge_indexes(status);
-- Background sync worker claims the next pending index by sync_status.
CREATE INDEX idx_knowledge_indexes_sync_status ON knowledge_indexes(sync_status);
-- Unique per org (same public_id allowed across orgs)
CREATE UNIQUE INDEX idx_knowledge_indexes_org_public_id
    ON knowledge_indexes(org_id, public_id);
-- Case-insensitive unique name per org while not deleted.
CREATE UNIQUE INDEX idx_knowledge_indexes_org_name_active
    ON knowledge_indexes(org_id, LOWER(name))
    WHERE status != 'deleted';

CREATE TRIGGER update_knowledge_indexes_updated_at BEFORE UPDATE ON knowledge_indexes
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE knowledge_indexes IS 'Knowledge Indexes — org-scoped source-backed embedded collections. See knowledge/runtime-resources/knowledge-indexes.md.';

-- ============================================
-- Index Documents (one ingested source document)
-- ============================================

CREATE TABLE knowledge_index_documents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    index_id UUID NOT NULL REFERENCES knowledge_indexes(id) ON DELETE CASCADE,
    public_id TEXT NOT NULL,

    -- Stable per-source locator (e.g. github://owner/repo@main/docs/x.md).
    source_uri TEXT NOT NULL,
    title VARCHAR(1024),
    mime_type VARCHAR(255),
    -- Drives incremental re-embedding: unchanged hash => skip re-embed.
    content_hash TEXT,
    size_bytes BIGINT,
    chunk_count INT NOT NULL DEFAULT 0,
    -- Stamped each sync pass; documents not seen in a pass are pruned.
    last_seen_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT knowledge_index_documents_public_id_format
        CHECK (public_id ~ '^kidoc_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_knowledge_index_documents_index_public_id
    ON knowledge_index_documents(index_id, public_id);
CREATE UNIQUE INDEX idx_knowledge_index_documents_index_source_uri
    ON knowledge_index_documents(index_id, source_uri);
CREATE INDEX idx_knowledge_index_documents_index_id
    ON knowledge_index_documents(index_id);

CREATE TRIGGER update_knowledge_index_documents_updated_at
    BEFORE UPDATE ON knowledge_index_documents
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE knowledge_index_documents IS 'Ingested source documents inside a Knowledge Index.';

-- ============================================
-- Index Chunks (the citable retrieval unit)
-- ============================================

CREATE TABLE knowledge_index_chunks (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    document_id UUID NOT NULL REFERENCES knowledge_index_documents(id) ON DELETE CASCADE,
    -- Denormalized for index-scoped queries and bulk delete.
    index_id UUID NOT NULL REFERENCES knowledge_indexes(id) ON DELETE CASCADE,
    public_id TEXT NOT NULL,

    ordinal INT NOT NULL,
    -- The chunk passage. Also written to the vector store for BM25 hybrid search.
    text TEXT NOT NULL,
    -- Provenance within the document (line / char / page ranges) for citations.
    location JSONB,
    token_count INT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT knowledge_index_chunks_public_id_format
        CHECK (public_id ~ '^kchk_[0-9a-f]{32}$')
);

-- public_id is the stable citation ID, unique within an index.
CREATE UNIQUE INDEX idx_knowledge_index_chunks_index_public_id
    ON knowledge_index_chunks(index_id, public_id);
CREATE INDEX idx_knowledge_index_chunks_document_id
    ON knowledge_index_chunks(document_id);
CREATE INDEX idx_knowledge_index_chunks_index_id
    ON knowledge_index_chunks(index_id);

COMMENT ON TABLE knowledge_index_chunks IS 'Citable chunks of an index document. Embedding vectors live in the external vector store, keyed by public_id.';
