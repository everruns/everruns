// Workspace Volume types

export type VolumeStatus = "active" | "archived" | "deleted";

export interface Volume {
  id: string;
  name: string;
  description?: string | null;
  status: VolumeStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateVolumeRequest {
  name: string;
  description?: string;
}

export interface UpdateVolumeRequest {
  name?: string;
  description?: string | null;
}

export interface ListVolumesParams {
  includeArchived?: boolean;
  search?: string;
}
