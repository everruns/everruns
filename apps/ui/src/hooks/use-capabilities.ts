// Capability hooks
//
// Note: Agent-specific capabilities are now part of the agent resource.
// Use useAgent() to get an agent with its capabilities included.
"use client";

import { useQuery } from "@tanstack/react-query";
import { getCapability, listCapabilities } from "@/lib/api/capabilities";
import type { CapabilityId } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export function useCapabilities() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: ["capabilities", org],
    queryFn: () => listCapabilities(),
    enabled: !!org,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCapability(capabilityId: CapabilityId | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: ["capability", capabilityId, org],
    queryFn: () => getCapability(capabilityId!),
    enabled: !!org && !!capabilityId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}
