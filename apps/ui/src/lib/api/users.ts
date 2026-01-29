// Users API client functions
import { api } from "./client";
import type { ListResponse, User, ListUsersQuery } from "./types";

/**
 * List all users with optional search
 */
export async function listUsers(query?: ListUsersQuery): Promise<User[]> {
  const params = new URLSearchParams();
  if (query?.search) {
    params.set("search", query.search);
  }

  const url = params.toString() ? `/v1/users?${params.toString()}` : `/v1/users`;

  const response = await api.get<ListResponse<User>>(url);
  return response.data.data;
}

interface SwitchOrgResponse {
  success: boolean;
  org_id: string;
}

/**
 * Switch the current organization.
 * Sets a server-side cookie (everruns_org) that will be sent with all subsequent requests.
 * This is the preferred method for org selection as cookies work with SSE (EventSource).
 */
export async function switchOrg(orgId: string): Promise<SwitchOrgResponse> {
  const response = await api.post<SwitchOrgResponse>("/v1/users/me/switch-org", {
    org_id: orgId,
  });
  return response.data;
}
