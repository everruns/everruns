// Feature flags API client
import { api } from "./client";
import type { FeatureFlags, OrgFeatureFlagSetting, OrgFeatureFlagsSettingsResponse } from "./types";

/** Deployment-level flags (pre-auth). Prefer org-scoped flags when an org is selected. */
export async function getFeatureFlags(): Promise<FeatureFlags> {
  const { data } = await api.get<FeatureFlags>("/v1/feature-flags");
  return data;
}

/** Effective flags for an organization (system gate + org opt-in). */
export async function getOrgFeatureFlags(orgId: string): Promise<FeatureFlags> {
  const { data } = await api.get<FeatureFlags>(`/v1/orgs/${orgId}/feature-flags`);
  return data;
}

export async function getOrgFeatureFlagSettings(
  orgId: string,
): Promise<OrgFeatureFlagsSettingsResponse> {
  const { data } = await api.get<OrgFeatureFlagsSettingsResponse>(
    `/v1/orgs/${orgId}/feature-flags/settings`,
  );
  return data;
}

export async function updateOrgFeatureFlags(
  orgId: string,
  flags: Record<string, boolean>,
): Promise<FeatureFlags> {
  const { data } = await api.patch<FeatureFlags>(`/v1/orgs/${orgId}/feature-flags`, { flags });
  return data;
}

export type { OrgFeatureFlagSetting, OrgFeatureFlagsSettingsResponse };
