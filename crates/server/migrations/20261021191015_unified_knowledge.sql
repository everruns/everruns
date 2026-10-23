-- Originals and extracted text live in the content store. These tables own
-- identities, visibility, resumable jobs and rebuildable lexical postings only.
CREATE TABLE knowledge_collections (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id),
 name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
 indexing TEXT NOT NULL DEFAULT 'off' CHECK (indexing IN ('off','lexical','hybrid')),
 embedding_model_id UUID REFERENCES models(id), indexer_id UUID,
 published_revision BIGINT NOT NULL DEFAULT 1, serving_config JSONB NOT NULL DEFAULT '{}',
 revision BIGINT NOT NULL DEFAULT 1, status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived')),
 rules JSONB NOT NULL DEFAULT '{}', created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(org_id,name), CHECK(indexing != 'hybrid' OR embedding_model_id IS NOT NULL)
);
CREATE TABLE knowledge_indexers (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), name TEXT NOT NULL,
 endpoint TEXT NOT NULL, credential_binding_id UUID, status TEXT NOT NULL DEFAULT 'active',
 revision BIGINT NOT NULL DEFAULT 1, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(org_id,name),
 FOREIGN KEY(org_id,credential_binding_id) REFERENCES resource_access(org_id,id) ON DELETE SET NULL (credential_binding_id)
);
ALTER TABLE knowledge_collections ADD FOREIGN KEY(indexer_id) REFERENCES knowledge_indexers(id);
CREATE TABLE knowledge_sources (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), collection_id UUID NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('managed','github','git','builtin')), root_path TEXT NOT NULL DEFAULT '', config JSONB NOT NULL DEFAULT '{}',
 connection_owner UUID, checkpoint TEXT, sync_status TEXT NOT NULL DEFAULT 'idle', last_error TEXT,
 last_synced_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX knowledge_source_roots ON knowledge_sources(collection_id,root_path) WHERE kind != 'managed';
CREATE UNIQUE INDEX knowledge_managed_source ON knowledge_sources(collection_id) WHERE kind = 'managed';
CREATE TABLE knowledge_files (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), collection_id UUID NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
 source_id UUID NOT NULL REFERENCES knowledge_sources(id) ON DELETE CASCADE, path TEXT NOT NULL,
 current_version UUID, indexed_version UUID, indexed_revision BIGINT, indexable BOOLEAN NOT NULL DEFAULT true, readable BOOLEAN NOT NULL DEFAULT true,
 seen_run UUID, deleted_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(collection_id,path), CHECK(path LIKE '/%' AND path NOT LIKE '%/../%' AND position(chr(92) in path)=0)
);
CREATE TABLE knowledge_versions (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), file_id UUID NOT NULL REFERENCES knowledge_files(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL, size_bytes BIGINT NOT NULL, mime_type TEXT NOT NULL,
 raw_key TEXT NOT NULL, raw_parts INT NOT NULL, text_key TEXT, text_bytes INT NOT NULL DEFAULT 0,
 metadata JSONB NOT NULL DEFAULT '{}', warnings JSONB NOT NULL DEFAULT '[]', extraction_status TEXT NOT NULL,
 source_uri TEXT NOT NULL, source_revision TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE knowledge_files ADD FOREIGN KEY(current_version) REFERENCES knowledge_versions(id);
ALTER TABLE knowledge_files ADD FOREIGN KEY(indexed_version) REFERENCES knowledge_versions(id);
CREATE TABLE knowledge_fragments (
 id TEXT PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), collection_id UUID NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
 file_id UUID NOT NULL REFERENCES knowledge_files(id) ON DELETE CASCADE, version_id UUID NOT NULL REFERENCES knowledge_versions(id) ON DELETE CASCADE,
 profile BIGINT NOT NULL, ordinal INT NOT NULL, byte_start INT NOT NULL, byte_length INT NOT NULL,
 terms TSVECTOR NOT NULL, UNIQUE(version_id,profile,ordinal)
);
CREATE INDEX knowledge_fragments_terms ON knowledge_fragments USING GIN(terms);
CREATE INDEX knowledge_fragments_scope ON knowledge_fragments(org_id,collection_id,version_id,profile);
CREATE INDEX knowledge_files_tree ON knowledge_files(collection_id,path text_pattern_ops) WHERE deleted_at IS NULL;
CREATE INDEX knowledge_files_source ON knowledge_files(source_id,path);
CREATE INDEX knowledge_versions_file ON knowledge_versions(file_id,created_at);
CREATE TABLE knowledge_jobs (
 id UUID PRIMARY KEY, org_id BIGINT NOT NULL REFERENCES organizations(org_id), collection_id UUID NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
 source_id UUID REFERENCES knowledge_sources(id) ON DELETE CASCADE, file_id UUID REFERENCES knowledge_files(id) ON DELETE CASCADE,
 version_id UUID REFERENCES knowledge_versions(id), kind TEXT NOT NULL CHECK(kind IN ('source','index','reindex')),
 revision BIGINT NOT NULL, status TEXT NOT NULL DEFAULT 'pending', lease_token UUID, lease_until TIMESTAMPTZ,
 attempts INT NOT NULL DEFAULT 0, cursor TEXT NOT NULL DEFAULT '', last_error TEXT,
 available_at TIMESTAMPTZ NOT NULL DEFAULT now(), created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX knowledge_jobs_active ON knowledge_jobs(collection_id,kind,source_id,file_id,version_id,revision) NULLS NOT DISTINCT WHERE status IN ('pending','running');
CREATE INDEX knowledge_jobs_due ON knowledge_jobs(available_at,created_at) WHERE status IN ('pending','running');
CREATE TABLE knowledge_retrievals (
 org_id BIGINT NOT NULL REFERENCES organizations(org_id), session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 input_id UUID NOT NULL, config_hash TEXT NOT NULL, evidence JSONB NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(session_id,input_id,config_hash)
);

CREATE TABLE knowledge_file_projections (
 org_id BIGINT NOT NULL REFERENCES organizations(org_id), collection_id UUID NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
 file_id UUID NOT NULL REFERENCES knowledge_files(id) ON DELETE CASCADE, revision BIGINT NOT NULL, version_id UUID NOT NULL REFERENCES knowledge_versions(id),
 PRIMARY KEY(file_id,revision)
);
CREATE INDEX knowledge_projection_lookup ON knowledge_file_projections(collection_id,revision,version_id);
