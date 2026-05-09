// Workspace Volume types

export type VolumeStatus = "active" | "archived" | "deleted";

export interface Volume {
  id: string;
  name: string;
  description?: string | null;
  source_type: VolumeSourceType;
  source: VolumeSource;
  is_readonly: boolean;
  sync_status: VolumeSyncStatus;
  last_synced_at?: string | null;
  last_sync_error?: string | null;
  status: VolumeStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export type VolumeSourceType = "manual" | "github" | "git";
export type VolumeSyncStatus = "idle" | "pending" | "syncing" | "synced" | "failed";

export type VolumeSource =
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

export type CreateVolumeSource =
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

export interface CreateVolumeRequest {
  name: string;
  description?: string;
  source?: CreateVolumeSource;
}

export interface UpdateVolumeRequest {
  name?: string;
  description?: string | null;
  source?: CreateVolumeSource;
}

export interface ListVolumesParams {
  includeArchived?: boolean;
  search?: string;
}
