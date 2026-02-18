// Organization members API functions
// Routes: GET/POST /v1/orgs/{org}/members, PATCH/DELETE /v1/orgs/{org}/members/{userId}

import { api } from "./client";
import type { OrgRole } from "./types";

export interface OrgMember {
  user_id: string;
  email: string;
  name: string;
  avatar_url: string | null;
  role: OrgRole;
  joined_at: string;
}

interface MembersListResponse {
  data: OrgMember[];
}

export async function listMembers(org: string): Promise<OrgMember[]> {
  const response = await api.get<MembersListResponse>(`/v1/orgs/${org}/members`);
  return response.data.data;
}

export async function addMember(
  org: string,
  userId: string,
  role: OrgRole = "member",
): Promise<OrgMember> {
  const response = await api.post<OrgMember>(`/v1/orgs/${org}/members`, {
    user_id: userId,
    role,
  });
  return response.data;
}

export async function updateMemberRole(
  org: string,
  userId: string,
  role: OrgRole,
): Promise<OrgMember> {
  const response = await api.patch<OrgMember>(`/v1/orgs/${org}/members/${userId}`, {
    role,
  });
  return response.data;
}

export async function removeMember(org: string, userId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/members/${userId}`);
}
