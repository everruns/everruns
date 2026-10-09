"use client";

import { getAgentActivity } from "@/lib/api/agent-activity";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";

/// Org-wide agent load, 24-hour run buckets and 7-day channel traffic.
/// Refreshes on a timer because "running now" is the point of the page.
export function useAgentActivity(options: { enabled?: boolean } = {}) {
  return useOrgScopedQuery({
    queryKey: queryKeys.agents.activity(),
    queryFn: getAgentActivity,
    enabled: options.enabled ?? true,
    refetchInterval: 30_000,
  });
}
