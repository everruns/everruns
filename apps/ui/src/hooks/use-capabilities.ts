// Capability hooks
//
// Note: Agent-specific capabilities are now part of the agent resource.
// Use useAgent() to get an agent with its capabilities included.

import { useQuery } from "@tanstack/react-query";
import {
  getCapability,
  listCapabilities,
} from "@/lib/api/capabilities";
import type { CapabilityId } from "@/lib/api/types";

export function useCapabilities() {
  return useQuery({
    queryKey: ["capabilities"],
    queryFn: listCapabilities,
  });
}

export function useCapability(capabilityId: CapabilityId | undefined) {
  return useQuery({
    queryKey: ["capability", capabilityId],
    queryFn: () => getCapability(capabilityId!),
    enabled: !!capabilityId,
  });
}
