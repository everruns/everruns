// Workspace Volumes API

import { api } from "./client";
import type {
  CreateVolumeRequest,
  ListResponse,
  ListVolumesParams,
  UpdateVolumeRequest,
  Volume,
} from "./types";

function volumesQuery(params: ListVolumesParams = {}): string {
  const query = new URLSearchParams();
  if (params.includeArchived) {
    query.set("include_archived", "true");
  }
  const search = params.search?.trim();
  if (search) {
    query.set("search", search);
  }
  const suffix = query.toString();
  return suffix ? `/v1/volumes?${suffix}` : "/v1/volumes";
}

export async function listVolumes(params: ListVolumesParams = {}): Promise<Volume[]> {
  const response = await api.get<ListResponse<Volume>>(volumesQuery(params));
  return response.data.data;
}

export async function getVolume(volumeId: string): Promise<Volume> {
  const response = await api.get<Volume>(`/v1/volumes/${volumeId}`);
  return response.data;
}

export async function createVolume(request: CreateVolumeRequest): Promise<Volume> {
  const response = await api.post<Volume>("/v1/volumes", request);
  return response.data;
}

export async function updateVolume(
  volumeId: string,
  request: UpdateVolumeRequest,
): Promise<Volume> {
  const response = await api.patch<Volume>(`/v1/volumes/${volumeId}`, request);
  return response.data;
}

export async function archiveVolume(volumeId: string): Promise<void> {
  await api.delete(`/v1/volumes/${volumeId}`);
}
