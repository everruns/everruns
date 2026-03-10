"use client";

// Feature flags provider
//
// Decision: Fetch flags once on mount via React Query (cached, no repeated queries).
// Decision: Default all flags to false while loading to avoid flash of gated content.
// Decision: Provider sits above OrgProvider so flags are available everywhere.

import { createContext, useContext, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { getFeatureFlags } from "@/lib/api/feature-flags";
import type { FeatureFlags } from "@/lib/api/types";

const DEFAULT_FLAGS: FeatureFlags = {
  global_chat: false,
  apps: false,
  global_search: false,
};

export interface FeatureFlagsContextValue {
  flags: FeatureFlags;
  isLoading: boolean;
}

const FeatureFlagsContext = createContext<FeatureFlagsContextValue | undefined>(undefined);

export function FeatureFlagsProvider({ children }: { children: ReactNode }) {
  const { data, isLoading } = useQuery({
    queryKey: ["feature-flags"],
    queryFn: getFeatureFlags,
    staleTime: 5 * 60 * 1000, // 5 min
    refetchOnWindowFocus: false,
  });

  const value: FeatureFlagsContextValue = {
    flags: data ?? DEFAULT_FLAGS,
    isLoading,
  };

  return <FeatureFlagsContext.Provider value={value}>{children}</FeatureFlagsContext.Provider>;
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
