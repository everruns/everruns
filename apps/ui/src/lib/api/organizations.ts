// Organization API functions
// Routes: POST /v1/orgs, GET/PATCH /v1/orgs/{org}

import { api } from "./client";
import type { CreateOrganizationRequest, Organization, UpdateOrganizationRequest } from "./types";

export async function createOrganization(data: CreateOrganizationRequest): Promise<Organization> {
  const response = await api.post<Organization>("/v1/orgs", data);
  return response.data;
}

export async function getOrganization(org: string): Promise<Organization> {
  const response = await api.get<Organization>(`/v1/orgs/${org}`);
  return response.data;
}

export async function updateOrganization(
  org: string,
  data: UpdateOrganizationRequest,
): Promise<Organization> {
  const response = await api.patch<Organization>(`/v1/orgs/${org}`, data);
  return response.data;
}
