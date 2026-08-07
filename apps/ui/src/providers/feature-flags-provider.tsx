"use client";

// Feature flags provider
//
// Decision: When an org is selected, fetch org-effective flags (system + opt-in).
// Decision: Fall back to deployment-level flags before org context is ready.
// Decision: Default all flags to false while loading to avoid flash of gated content.

import { createContext, useContext, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { getFeatureFlags, getOrgFeatureFlags } from "@/lib/api/feature-flags";
import type { FeatureFlags } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

const DEFAULT_FLAGS: FeatureFlags = {
  global_chat: false,
  notifications: false,
  evals: false,
  app_budgets: false,
  agent_versions: false,
  voice: false,
  agent_delegation: false,
  observers: false,
  public_chat: false,
  mcp_endpoint: false,
  webmcp: false,
};

export interface FeatureFlagsContextValue {
  flags: FeatureFlags;
  isLoading: boolean;
}

const FeatureFlagsContext = createContext<FeatureFlagsContextValue | undefined>(undefined);

function FeatureFlagsLoader({ children }: { children: ReactNode }) {
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id;

  const systemQuery = useQuery({
    queryKey: ["feature-flags", "system"],
    queryFn: getFeatureFlags,
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
    enabled: !orgId,
  });

  const orgQuery = useQuery({
    queryKey: ["feature-flags", "org", orgId],
    queryFn: () => getOrgFeatureFlags(orgId!),
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
    enabled: Boolean(orgId),
  });

  const activeQuery = orgId ? orgQuery : systemQuery;
  const value: FeatureFlagsContextValue = {
    flags: activeQuery.data ?? DEFAULT_FLAGS,
    isLoading: activeQuery.isLoading,
  };

  return <FeatureFlagsContext.Provider value={value}>{children}</FeatureFlagsContext.Provider>;
}

export function FeatureFlagsProvider({ children }: { children: ReactNode }) {
  return <FeatureFlagsLoader>{children}</FeatureFlagsLoader>;
}

export function useFeatureFlags(): FeatureFlags {
  const context = useContext(FeatureFlagsContext);
  if (context === undefined) {
    throw new Error("useFeatureFlags must be used within a FeatureFlagsProvider");
  }
  return context.flags;
}

export function useFeatureFlag(flag: keyof FeatureFlags): boolean {
  const flags = useFeatureFlags();
  return flags[flag];
}
