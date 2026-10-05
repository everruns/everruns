"use client";

import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getSandbox,
  getSandboxFleetStats,
  getSandboxTimeline,
  listSandboxes,
  manageSessionSandbox,
  type SandboxAction,
  type SandboxFleetFilters,
} from "@/lib/api/sandboxes";

const fleetKey = ["sandbox-fleet"] as const;

// Live Sandboxes change state on their own (idle pause, loss), so the fleet
// refreshes while the page is open.
const REFRESH_MS = 15_000;

export function useSandboxFleet(
  filters: SandboxFleetFilters,
  page: { limit: number; offset: number },
) {
  return useQuery({
    queryKey: [...fleetKey, "list", filters, page],
    queryFn: () => listSandboxes(filters, page),
    placeholderData: keepPreviousData,
    refetchInterval: REFRESH_MS,
  });
}

/**
 * Roll-ups ignore the state and attention filters: the summary chips are how
 * those filters are chosen, so they must keep showing the whole distribution.
 */
export function useSandboxFleetStats(filters: SandboxFleetFilters) {
  const scope = { provider: filters.provider, search: filters.search };
  return useQuery({
    queryKey: [...fleetKey, "stats", scope],
    queryFn: () => getSandboxFleetStats(scope),
    placeholderData: keepPreviousData,
    refetchInterval: REFRESH_MS,
  });
}

export function useSandboxTimeline(
  filters: SandboxFleetFilters,
  window: { from: string; to: string },
  enabled = true,
) {
  return useQuery({
    queryKey: [...fleetKey, "timeline", filters, window],
    queryFn: () => getSandboxTimeline(filters, window),
    placeholderData: keepPreviousData,
    enabled,
  });
}

export function useSandbox(id: string | null) {
  return useQuery({
    queryKey: [...fleetKey, "detail", id],
    queryFn: () => getSandbox(id as string),
    enabled: Boolean(id),
  });
}

export function useManageSandbox() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId, action }: { sessionId: string; action: SandboxAction }) =>
      manageSessionSandbox(sessionId, action),
    onSuccess: (_data, { sessionId }) => {
      void queryClient.invalidateQueries({ queryKey: fleetKey });
      void queryClient.invalidateQueries({
        queryKey: ["session-sandbox", sessionId],
      });
    },
  });
}
