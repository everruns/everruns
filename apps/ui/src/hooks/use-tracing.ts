"use client";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { listTracingDestinations, getTracingPolicy } from "@/lib/api/tracing";
export function useTracingDestinations() {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["tracing", currentOrg?.public_id, "destinations"],
    enabled: !!currentOrg,
    queryFn: listTracingDestinations,
    refetchInterval: 30_000,
  });
}
export function useTracingPolicy(agent?: string) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["tracing", currentOrg?.public_id, "policy", agent ?? "defaults"],
    enabled: !!currentOrg,
    queryFn: () => getTracingPolicy(agent),
  });
}
export function useRefreshTracing() {
  const client = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return () => client.invalidateQueries({ queryKey: ["tracing", org] });
}
