// Organization API functions
// Routes: POST /v1/orgs, GET/PATCH /v1/orgs/{org},
//         POST /v1/orgs/{org}/onboarding/complete,
//         GET/PUT /v1/orgs/{org}/egress-allowlist, PUT /v1/orgs/{org}/egress-allowlist/grant

import { api } from "./client";
import type {
  CreateOrganizationRequest,
  Organization,
  OrgEgressAllowlistResponse,
  UpdateOrganizationRequest,
} from "./types";

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

/**
 * Durably mark an org's onboarding wizard as finished or skipped. Idempotent
 * server-side (the completion timestamp is set only once). The setup page calls
 * this before showing the Done step so a skip is remembered on re-entry.
 */
export async function completeOrgOnboarding(org: string): Promise<Organization> {
  const response = await api.post<Organization>(`/v1/orgs/${org}/onboarding/complete`, {});
  return response.data;
}

/** The org's outbound allowlist extension and what the caller may do with it. */
export async function getOrgEgressAllowlist(org: string): Promise<OrgEgressAllowlistResponse> {
  const response = await api.get<OrgEgressAllowlistResponse>(`/v1/orgs/${org}/egress-allowlist`);
  return response.data;
}

/** Replace the org's extension patterns (org admin; requires a platform grant). */
export async function setOrgEgressAllowlist(
  org: string,
  patterns: string[],
): Promise<OrgEgressAllowlistResponse> {
  const response = await api.put<OrgEgressAllowlistResponse>(`/v1/orgs/${org}/egress-allowlist`, {
    patterns,
  });
  return response.data;
}

/** Grant or revoke the org's right to extend the allowlist (platform users only). */
export async function setOrgEgressAllowlistGrant(
  org: string,
  granted: boolean,
): Promise<OrgEgressAllowlistResponse> {
  const response = await api.put<OrgEgressAllowlistResponse>(
    `/v1/orgs/${org}/egress-allowlist/grant`,
    { granted },
  );
  return response.data;
}
