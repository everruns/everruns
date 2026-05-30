import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getOrgFeatureFlagSettings,
  getOrgFeatureFlags,
  updateOrgFeatureFlags,
} from "@/lib/api/feature-flags";
import { useOrg } from "@/providers/org-provider";

export function useOrgFeatureFlags() {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.id;

  return useQuery({
    queryKey: ["org-feature-flags", orgId],
    queryFn: () => getOrgFeatureFlags(orgId!),
    enabled: Boolean(orgId),
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
  });
}

export function useOrgFeatureFlagSettings() {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.id;

  return useQuery({
    queryKey: ["org-feature-flags-settings", orgId],
    queryFn: () => getOrgFeatureFlagSettings(orgId!),
    enabled: Boolean(orgId),
    staleTime: 60 * 1000,
  });
}

export function useUpdateOrgFeatureFlags() {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.id;
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (flags: Record<string, boolean>) => updateOrgFeatureFlags(orgId!, flags),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["org-feature-flags", orgId] });
      queryClient.invalidateQueries({ queryKey: ["org-feature-flags-settings", orgId] });
      queryClient.invalidateQueries({ queryKey: ["feature-flags"] });
    },
  });
}
