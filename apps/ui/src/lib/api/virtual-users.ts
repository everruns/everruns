import { api } from "./client";
import type {
  VirtualUser,
  CreateVirtualUserRequest,
  PaginatedResponse,
  UpdateVirtualUserRequest,
} from "./types";

export async function listVirtualUsers(
  includeArchived = false,
  options: { offset?: number; usage?: "end_user" | "service"; search?: string } = {},
): Promise<PaginatedResponse<VirtualUser>> {
  const params = new URLSearchParams({
    include_archived: String(includeArchived),
    limit: "100",
    offset: String(options.offset ?? 0),
  });
  if (options.usage) params.set("usage", options.usage);
  if (options.search) params.set("search", options.search);
  const response = await api.get<PaginatedResponse<VirtualUser>>(`/v1/virtual-users?${params}`);
  return response.data;
}

export async function getVirtualUser(identityId: string): Promise<VirtualUser> {
  const response = await api.get<VirtualUser>(`/v1/virtual-users/${identityId}`);
  return response.data;
}

export async function createVirtualUser(request: CreateVirtualUserRequest): Promise<VirtualUser> {
  const response = await api.post<VirtualUser>("/v1/virtual-users", request);
  return response.data;
}

export async function updateVirtualUser(
  identityId: string,
  request: UpdateVirtualUserRequest,
): Promise<VirtualUser> {
  const response = await api.patch<VirtualUser>(`/v1/virtual-users/${identityId}`, request);
  return response.data;
}

export async function deleteVirtualUser(identityId: string): Promise<void> {
  await api.delete(`/v1/virtual-users/${identityId}`);
}

export async function destroyVirtualUser(identityId: string): Promise<void> {
  await api.post(`/v1/virtual-users/${identityId}/delete`);
}
