"use client";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useResourceOrgFallback } from "./use-resource-org-fallback";
import { listApplications, getApplication, listApplicationLaunchers } from "@/lib/api/applications";
export function useApplications(includeArchived = false) {
  return useOrgScopedQuery({
    queryKey: ["applications", { includeArchived }],
    queryFn: () => listApplications(includeArchived),
  });
}
export function useApplication(id: string) {
  const query = useOrgScopedQuery({
    queryKey: ["applications", id],
    queryFn: () => getApplication(id),
  });
  const fallback = useResourceOrgFallback({
    resourceId: id,
    error: query.error,
    isLoading: query.isLoading,
  });
  return { ...query, isLoading: query.isLoading || fallback.isCheckingOtherOrgs };
}
export function useApplicationLaunchers() {
  return useOrgScopedQuery({
    queryKey: ["application-launchers"],
    queryFn: listApplicationLaunchers,
  });
}
