// Cross-org resource resolver client.
// See specs/multitenancy.md (Cross-Org Resource Resolution).

import { ApiError, api } from "./client";

export interface ResolveOrgResult {
  org_id: string;
  org_name: string;
}

/**
 * Resolve the owning organization for a prefixed resource ID.
 *
 * Returns `null` when the backend answers 404 — either the id is unknown, its
 * prefix is not registered, or the resource lives in an org the caller is not
 * a member of. Any other error is thrown.
 */
export async function resolveOrgForResource(id: string): Promise<ResolveOrgResult | null> {
  try {
    const response = await api.get<ResolveOrgResult>(
      `/v1/resolve-org?id=${encodeURIComponent(id)}`,
    );
    return response.data;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}
