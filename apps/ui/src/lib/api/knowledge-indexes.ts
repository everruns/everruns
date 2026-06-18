// Knowledge Index API
//
// Mirrors src/lib/api/memory.ts conventions (fetch wrapper, typed CRUD + sync).

import { api } from "./client";
import type {
  CreateKnowledgeIndexRequest,
  KnowledgeIndex,
  KnowledgeIndexDocument,
  ListKnowledgeIndexesParams,
  ListResponse,
  UpdateKnowledgeIndexRequest,
} from "./types";

function knowledgeIndexQuery(params: ListKnowledgeIndexesParams = {}): string {
  const query = new URLSearchParams();
  if (params.includeArchived) {
    query.set("include_archived", "true");
  }
  const search = params.search?.trim();
  if (search) {
    query.set("search", search);
  }
  const suffix = query.toString();
  return suffix ? `/v1/knowledge-indexes?${suffix}` : "/v1/knowledge-indexes";
}

export async function listKnowledgeIndexes(
  params: ListKnowledgeIndexesParams = {},
): Promise<KnowledgeIndex[]> {
  const response = await api.get<ListResponse<KnowledgeIndex>>(knowledgeIndexQuery(params));
  return response.data.data;
}

export async function getKnowledgeIndex(indexId: string): Promise<KnowledgeIndex> {
  const response = await api.get<KnowledgeIndex>(`/v1/knowledge-indexes/${indexId}`);
  return response.data;
}

export async function createKnowledgeIndex(
  request: CreateKnowledgeIndexRequest,
): Promise<KnowledgeIndex> {
  const response = await api.post<KnowledgeIndex>("/v1/knowledge-indexes", request);
  return response.data;
}

export async function updateKnowledgeIndex(
  indexId: string,
  request: UpdateKnowledgeIndexRequest,
): Promise<KnowledgeIndex> {
  const response = await api.patch<KnowledgeIndex>(`/v1/knowledge-indexes/${indexId}`, request);
  return response.data;
}

export async function syncKnowledgeIndexNow(indexId: string): Promise<KnowledgeIndex> {
  const response = await api.post<KnowledgeIndex>(`/v1/knowledge-indexes/${indexId}/sync`);
  return response.data;
}

export async function archiveKnowledgeIndex(indexId: string): Promise<void> {
  await api.delete(`/v1/knowledge-indexes/${indexId}`);
}

export async function listKnowledgeIndexDocuments(
  indexId: string,
): Promise<KnowledgeIndexDocument[]> {
  const response = await api.get<ListResponse<KnowledgeIndexDocument>>(
    `/v1/knowledge-indexes/${indexId}/documents`,
  );
  return response.data.data;
}
