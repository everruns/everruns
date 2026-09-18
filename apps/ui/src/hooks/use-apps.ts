"use client";

import { getApp, getApps } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

interface UseAppsOptions {
  includeArchived?: boolean;
  enabled?: boolean;
}

export function useApps(options: UseAppsOptions = {}) {
  const includeArchived = options.includeArchived ?? false;
  return useOrgScopedQuery({
    queryKey: queryKeys.apps.list(includeArchived),
    queryFn: () => getApps(includeArchived),
    enabled: options.enabled ?? true,
  });
}

export function useApp(appId: string | undefined) {
  const query = useOrgScopedQuery({
    queryKey: queryKeys.apps.detail(appId ?? ""),
    queryFn: () => getApp(appId!),
    enabled: !!appId,
  });
  const fallback = useResourceOrgFallback({
    resourceId: appId,
    error: query.error,
    isLoading: query.isLoading,
  });
  return {
    ...query,
    isLoading: query.isLoading || fallback.isCheckingOtherOrgs,
  };
}
