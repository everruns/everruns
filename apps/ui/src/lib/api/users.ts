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

  const url = params.toString()
    ? `/v1/users?${params.toString()}`
    : `/v1/users`;

  const response = await api.get<ListResponse<User>>(url);
  return response.data.data;
}
