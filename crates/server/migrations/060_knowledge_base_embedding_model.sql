-- Knowledge base embedding model configuration (EVE-phase6).
-- Resolves the open question in specs/knowledge-bases.md: embedding-backed
-- hybrid retrieval is configured per knowledge base, not org-wide.
-- An org can configure different embedding models for different KBs
-- (e.g. multilingual embeddings for a multilingual KB).

ALTER TABLE knowledge_bases
    ADD COLUMN embedding_model_id UUID REFERENCES llm_models(id) ON DELETE SET NULL;

COMMENT ON COLUMN knowledge_bases.embedding_model_id IS
    'Optional embedding model for hybrid retrieval. NULL = keyword search only. See specs/knowledge-bases.md.';
