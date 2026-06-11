// Memory types

export type MemoryStatus = "active" | "archived" | "deleted";

export interface Memory {
  id: string;
  name: string;
  description?: string | null;
  source_type: MemorySourceType;
  source: MemorySource;
  is_readonly: boolean;
  sync_status: MemorySyncStatus;
  last_synced_at?: string | null;
  last_sync_error?: string | null;
  status: MemoryStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export type MemorySourceType = "manual" | "github" | "git";
export type MemorySyncStatus = "idle" | "pending" | "syncing" | "synced" | "failed";

export type MemorySource =
  | {
      provider: "manual";
    }
  | {
      provider: "github";
      repository: string;
      branch: string;
      root_folder?: string | null;
      sync_interval_secs?: number | null;
    }
  | {
      provider: "git";
      url: string;
      branch: string;
      root_folder?: string | null;
      sync_interval_secs?: number | null;
    };

export type CreateMemorySource =
  | {
      type: "github";
      repository: string;
      branch?: string;
      root_folder?: string;
      sync_interval_secs?: number;
    }
  | {
      type: "git";
      url: string;
      branch?: string;
      root_folder?: string;
      sync_interval_secs?: number;
    };

export interface CreateMemoryRequest {
  name: string;
  description?: string;
  source?: CreateMemorySource;
}

export interface UpdateMemoryRequest {
  name?: string;
  description?: string | null;
  source?: CreateMemorySource;
}

export interface ListMemoriesParams {
  includeArchived?: boolean;
  search?: string;
}
