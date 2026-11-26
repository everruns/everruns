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
  return suffix ? `/v1/platform/memories?${suffix}` : "/v1/platform/memories";
}

export async function listMemories(params: ListMemoriesParams = {}): Promise<Memory[]> {
  const response = await api.get<ListResponse<Memory>>(memoryQuery(params));
  return response.data.data;
}

export async function getMemory(memoryId: string): Promise<Memory> {
  const response = await api.get<Memory>(`/v1/platform/memories/${memoryId}`);
  return response.data;
}

export async function createMemory(request: CreateMemoryRequest): Promise<Memory> {
  const response = await api.post<Memory>("/v1/platform/memories", request);
  return response.data;
}

export async function updateMemory(
  memoryId: string,
  request: UpdateMemoryRequest,
): Promise<Memory> {
  const response = await api.patch<Memory>(`/v1/platform/memories/${memoryId}`, request);
  return response.data;
}

export async function syncMemoryNow(memoryId: string): Promise<Memory> {
  const response = await api.post<Memory>(`/v1/platform/memories/${memoryId}/sync`);
  return response.data;
}

export async function archiveMemory(memoryId: string): Promise<void> {
  await api.delete(`/v1/platform/memories/${memoryId}`);
}

// Memory v2 views (knowledge/runtime-resources/memory.md): a memory row is a
// store, its typed entries are Markdown files in kind folders.

export type MemoryEntryKind = "decision" | "rule" | "fact" | "preference" | "blocked" | "note";

export interface MemoryStoreKeyView {
  scope: "agent" | "user" | "org" | "team" | "parent" | "named";
  name?: string | null;
}

export interface MemoryStoreView extends Memory {
  /** `org` (the organization's or a named memory), `agent`, `team` or `user`. */
  scope: "org" | "agent" | "team" | "user";
  /** How agents name it: `agent`, `user`, `org`, `team`, or the memory's name. */
  store: string;
  mount_path: string;
  about: string;
  /** The owning agent's name, for an agent or team memory. */
  owner_name?: string | null;
  entry_count: number;
  file_count: number;
  blocked_count: number;
  proposal_count: number;
  last_changed_at: string;
}

export interface MemoryEntrySource {
  kind: string;
  reference?: string | null;
  url?: string | null;
}

export interface MemoryEntry {
  store: MemoryStoreKeyView;
  path: string;
  kind: MemoryEntryKind;
  subject?: string | null;
  statement: string;
  quote?: string | null;
  source?: MemoryEntrySource | null;
  occurred_at: string;
  recorded_at: string;
  recorded_by: string;
  resolved_at?: string | null;
  resolution?: string | null;
  proposed?: boolean;
}

export interface MemoryEntriesView {
  store: string;
  entries: MemoryEntry[];
  invalid: { path: string; error: string }[];
}

export interface MemoryFileVersion {
  path: string;
  action: "write" | "delete";
  content?: string | null;
  size_bytes: number;
  changed_by?: string | null;
  created_at: string;
}

export async function listMemoryStores(): Promise<MemoryStoreView[]> {
  const response = await api.get<ListResponse<MemoryStoreView>>("/v1/platform/memory/stores");
  return response.data.data;
}

export async function listMemoryEntries(memoryId: string): Promise<MemoryEntriesView> {
  const response = await api.get<MemoryEntriesView>(`/v1/platform/memories/${memoryId}/entries`);
  return response.data;
}

export async function listMemoryHistory(
  memoryId: string,
  path?: string,
): Promise<MemoryFileVersion[]> {
  const query = path ? `?path=${encodeURIComponent(path)}` : "";
  const response = await api.get<ListResponse<MemoryFileVersion>>(
    `/v1/platform/memories/${memoryId}/history${query}`,
  );
  return response.data.data;
}
