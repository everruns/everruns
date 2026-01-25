// Organization API functions
// Routes: GET/PATCH /v1/orgs/{org}

import { api } from "./client";
import type { Organization, UpdateOrganizationRequest } from "./types";

export async function getOrganization(org: string): Promise<Organization> {
  const response = await api.get<Organization>(`/v1/orgs/${org}`);
  return response.data;
}

export async function updateOrganization(
  org: string,
  data: UpdateOrganizationRequest
): Promise<Organization> {
  const response = await api.patch<Organization>(`/v1/orgs/${org}`, data);
  return response.data;
}
