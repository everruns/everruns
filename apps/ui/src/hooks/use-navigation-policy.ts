"use client";

import { useCallback } from "react";
import { usePolicies } from "@/hooks/use-policies";

/**
 * Resolves `NavigationItem.policy` for the sidebar, Settings, and command search.
 * MCP and model catalog policies gate destinations; other IDs fail closed.
 */
export function useNavigationPolicy(): (policy: string) => boolean {
  const mcp = usePolicies("mcp-servers");
  const models = usePolicies("models");
  const canMcp = mcp.can;
  const canModels = models.can;
  return useCallback(
    (policy: string) =>
      policy.startsWith("mcp_server.")
        ? canMcp(policy)
        : policy.startsWith("model.")
          ? canModels(policy)
          : false,
    [canMcp, canModels],
  );
}
