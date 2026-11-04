import { api } from "./client";
import { withOrgHeader } from "./active-org";

export type IndexMode = "off" | "lexical" | "hybrid";
export interface KnowledgeCollection {
  id: string;
  name: string;
  description: string;
  indexing: IndexMode;
  embedding_model_id: string | null;
  indexer_id: string | null;
  revision: number;
  published_revision: number;
  status: string;
  rules: Record<string, unknown>;
  file_count?: number;
  indexed_count?: number;
  sources?: KnowledgeSource[];
  jobs?: { id: string; kind: string; status: string; error?: string }[];
}
export interface KnowledgeSource {
  id: string;
  kind: string;
  root_path: string;
  config: Record<string, string>;
  sync_status: string;
  last_error: string | null;
  last_synced_at: string | null;
}
export interface KnowledgeFile {
  id: string;
  path: string;
  current_version: string;
  indexed_version: string | null;
  indexable: boolean;
  readable: boolean;
  source_id: string;
  version?: {
    size_bytes: number;
    mime_type: string;
    extraction_status: string;
    metadata: unknown;
    warnings: string[];
    created_at?: string;
  };
}
export interface KnowledgeIndexer {
  id: string;
  name: string;
  endpoint: string;
  credential_binding_id: string | null;
  status: string;
}
export interface KnowledgeHit {
  id: string;
  collection_id: string;
  file_id: string;
  version_id: string;
  path: string;
  title: string;
  snippet: string;
  source_uri: string;
  location: unknown;
}
export interface KnowledgeResults {
  results: KnowledgeHit[];
  warnings: string[];
  truncated: boolean;
}
export interface CursorPage<T> {
  data: T[];
  next_cursor: string | null;
}
export interface CollectionInput {
  name: string;
  description: string;
  indexing: IndexMode;
  embedding_model_id?: string | null;
  indexer_id?: string | null;
  rules?: Record<string, unknown>;
}
export const listKnowledge = async (after = "") =>
  (
    await api.get<CursorPage<KnowledgeCollection>>(
      `/v1/knowledge?limit=100&after=${encodeURIComponent(after)}`,
    )
  ).data;
export const getKnowledge = async (id: string) =>
  (await api.get<KnowledgeCollection>(`/v1/knowledge/${id}`)).data;
export const createKnowledge = async (input: CollectionInput) =>
  (await api.post<KnowledgeCollection>("/v1/knowledge", input)).data;
export const updateKnowledge = async (id: string, input: CollectionInput) =>
  (await api.patch<KnowledgeCollection>(`/v1/knowledge/${id}`, input)).data;
/** One entry of a folder listing: a file, or a subfolder (`kind: "folder"`). */
export type KnowledgeEntry =
  | (KnowledgeFile & { kind: "file"; name: string })
  | { kind: "folder"; path: string; name: string };
/** The immediate children of a folder, paged by cursor. */
export const listKnowledgeFolder = async (id: string, path = "/", after = "") =>
  (
    await api.get<CursorPage<KnowledgeEntry>>(
      `/v1/knowledge/${id}/files?${new URLSearchParams({ path, after, limit: "500", tree: "true" })}`,
    )
  ).data;
export const deleteKnowledgeFile = async (id: string, path: string) =>
  api.delete(`/v1/knowledge/${id}/files?${new URLSearchParams({ path })}`);
export const syncKnowledge = async (id: string) => api.post(`/v1/knowledge/${id}/sync`);
export const addKnowledgeSource = async (
  id: string,
  input: {
    kind: string;
    root_path: string;
    config: Record<string, string>;
    connection_owner?: string;
  },
) => api.post(`/v1/knowledge/${id}/sources`, input);
export const searchKnowledge = async (id: string, query: string) =>
  (await api.post<KnowledgeResults>("/v1/knowledge/search", { collections: [id], query, limit: 8 }))
    .data;
export const listKnowledgeIndexers = async () =>
  (await api.get<{ data: KnowledgeIndexer[] }>("/v1/knowledge-indexers")).data.data;
export const createKnowledgeIndexer = async (input: {
  name: string;
  endpoint: string;
  credential_binding_id: string | null;
}) => api.post("/v1/knowledge-indexers", input);
/** Writes a text file, typed by its extension (Markdown unless it says otherwise). */
export const writeKnowledgeFile = async (id: string, path: string, content: string) => {
  const ext = path.split(".").pop()?.toLowerCase();
  const mime =
    ext === "json"
      ? "application/json"
      : ext === "txt" || ext === "csv" || ext === "yaml" || ext === "yml"
        ? "text/plain"
        : "text/markdown";
  return api.put(`/v1/knowledge/${id}/files`, { path, content, mime_type: mime });
};
export async function uploadKnowledgeFile(id: string, file: File, path: string) {
  const result = await fetch(
    `/api/v1/knowledge/${id}/upload?${new URLSearchParams({ path, mime_type: file.type || "application/octet-stream" })}`,
    { method: "PUT", headers: withOrgHeader(), credentials: "include", body: file },
  );
  if (!result.ok) throw new Error("Upload failed. Check your permissions and the file path.");
  return result.json();
}
export const updateKnowledgeFileMetadata = async (
  id: string,
  input: {
    path: string;
    metadata?: Record<string, unknown>;
    indexable?: boolean;
    readable?: boolean;
  },
) =>
  (await api.patch<{ file_id: string; version_id: string }>(`/v1/knowledge/${id}/files`, input))
    .data;
