// Org-scoped memory store types (see specs/memory.md)

export type MemoryKind = "fact" | "preference" | "correction" | "procedure" | "context";

export interface MemoryStore {
  id: string;
  name: string;
  is_default: boolean;
  active_memory_count: number;
  created_at: string;
  updated_at: string;
}

export interface Memory {
  id: string;
  store_id: string;
  content: string;
  content_parts: unknown;
  kind: string;
  importance: number;
  tags: string[];
  active: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateMemoryStoreRequest {
  name: string;
  is_default?: boolean;
}

export interface ListMemoriesParams {
  query?: string;
  kind?: string;
  tag?: string[];
  includeInactive?: boolean;
  limit?: number;
}

export interface ListMemoriesResponse {
  data: Memory[];
  total: number;
}
