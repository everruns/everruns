// Memory API

import { api } from "./client";
import type {
  CreateMemoryRequest,
  ListResponse,
  ListMemoriesParams,
  UpdateMemoryRequest,
  Memory,
} from "./types";

function memoryQuery(params: ListMemoriesParams = {}): string {
  const query = new URLSearchParams();
  if (params.includeArchived) {
    query.set("include_archived", "true");
  }
  const search = params.search?.trim();
  if (search) {
    query.set("search", search);
  }
  const suffix = query.toString();
  return suffix ? `/v1/memory?${suffix}` : "/v1/memory";
}

export async function listMemories(params: ListMemoriesParams = {}): Promise<Memory[]> {
  const response = await api.get<ListResponse<Memory>>(memoryQuery(params));
  return response.data.data;
}

export async function getMemory(memoryId: string): Promise<Memory> {
  const response = await api.get<Memory>(`/v1/memory/${memoryId}`);
  return response.data;
}

export async function createMemory(request: CreateMemoryRequest): Promise<Memory> {
  const response = await api.post<Memory>("/v1/memory", request);
  return response.data;
}

export async function updateMemory(
  memoryId: string,
  request: UpdateMemoryRequest,
): Promise<Memory> {
  const response = await api.patch<Memory>(`/v1/memory/${memoryId}`, request);
  return response.data;
}

export async function syncMemoryNow(memoryId: string): Promise<Memory> {
  const response = await api.post<Memory>(`/v1/memory/${memoryId}/sync`);
  return response.data;
}

export async function archiveMemory(memoryId: string): Promise<void> {
  await api.delete(`/v1/memory/${memoryId}`);
}
