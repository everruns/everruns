"use client";

import { useCallback } from "react";
import { usePolicies } from "@/hooks/use-policies";

/**
 * Resolves `NavigationItem.policy` for the sidebar, Settings, and command search.
 * Only MCP catalog policies gate a destination today; any other id fails closed.
 */
export function useNavigationPolicy(): (policy: string) => boolean {
  const mcp = usePolicies("mcp-servers");
  const canMcp = mcp.can;
  return useCallback(
    (policy: string) => (policy.startsWith("mcp_server.") ? canMcp(policy) : false),
    [canMcp],
  );
}
