// Knowledge Index types
//
// Mirrors crates/server/src/domains/knowledge_indexes/types.rs:
// KnowledgeIndexResponse / KnowledgeIndexDocumentResponse and the request bodies.

export type KnowledgeIndexStatus = "active" | "archived" | "deleted";
export type KnowledgeIndexSourceType = "github" | "git";
export type KnowledgeIndexSyncStatus = "idle" | "pending" | "syncing" | "synced" | "failed";

export interface KnowledgeIndex {
  id: string;
  name: string;
  description?: string | null;
  source_type: KnowledgeIndexSourceType;
  /** Non-secret source coordinates. Never holds credentials. */
  source_config: Record<string, unknown>;
  embedding_model_id: string;
  vector_dim?: number | null;
  status: KnowledgeIndexStatus;
  sync_status: KnowledgeIndexSyncStatus;
  last_synced_at?: string | null;
  last_sync_error?: string | null;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface KnowledgeIndexDocument {
  id: string;
  index_id: string;
  source_uri: string;
  title?: string | null;
  mime_type?: string | null;
  content_hash?: string | null;
  size_bytes?: number | null;
  chunk_count: number;
  last_seen_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateKnowledgeIndexRequest {
  name: string;
  description?: string;
  /** Defaults to "github" on the server. */
  source_type?: KnowledgeIndexSourceType;
  source_config?: Record<string, unknown>;
  /** Required. Must resolve to a model whose driver declares Embeddings. */
  embedding_model_id: string;
}

export interface UpdateKnowledgeIndexRequest {
  name?: string;
  /** Set to null to clear the description. */
  description?: string | null;
  source_config?: Record<string, unknown>;
  /** Required on the server; cannot be cleared. */
  embedding_model_id?: string;
}

export interface ListKnowledgeIndexesParams {
  includeArchived?: boolean;
  search?: string;
}
