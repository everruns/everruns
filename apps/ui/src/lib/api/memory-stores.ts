// Org-scoped memory store API client (see specs/memory.md)

import { api } from "./client";
import type {
  CreateMemoryStoreRequest,
  ListMemoriesParams,
  ListMemoriesResponse,
  ListResponse,
  MemoryStore,
  UpdateMemoryStoreRequest,
} from "./types";

export async function listMemoryStores(): Promise<MemoryStore[]> {
  const response = await api.get<ListResponse<MemoryStore>>("/v1/memory-stores");
  return response.data.data;
}

export async function getMemoryStore(storeId: string): Promise<MemoryStore> {
  const response = await api.get<MemoryStore>(`/v1/memory-stores/${storeId}`);
  return response.data;
}

export async function createMemoryStore(request: CreateMemoryStoreRequest): Promise<MemoryStore> {
  const response = await api.post<MemoryStore>("/v1/memory-stores", request);
  return response.data;
}

export async function updateMemoryStore(
  storeId: string,
  request: UpdateMemoryStoreRequest,
): Promise<MemoryStore> {
  const response = await api.patch<MemoryStore>(`/v1/memory-stores/${storeId}`, request);
  return response.data;
}

function memoriesQuery(params: ListMemoriesParams = {}): string {
  const query = new URLSearchParams();
  if (params.query?.trim()) query.set("query", params.query.trim());
  if (params.kind) query.set("kind", params.kind);
  if (params.tag) {
    for (const t of params.tag) {
      query.append("tag", t);
    }
  }
  if (params.includeInactive) query.set("include_inactive", "true");
  if (params.limit) query.set("limit", String(params.limit));
  return query.toString();
}

export async function listMemories(
  storeId: string,
  params: ListMemoriesParams = {},
): Promise<ListMemoriesResponse> {
  const suffix = memoriesQuery(params);
  const url = suffix
    ? `/v1/memory-stores/${storeId}/memories?${suffix}`
    : `/v1/memory-stores/${storeId}/memories`;
  const response = await api.get<ListMemoriesResponse>(url);
  return response.data;
}

export async function forgetMemory(storeId: string, memoryId: string): Promise<void> {
  await api.delete(`/v1/memory-stores/${storeId}/memories/${memoryId}`);
}
