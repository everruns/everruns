"use client";

import { useQuery } from "@tanstack/react-query";
import { listEnvironmentTargets } from "@/lib/api/environments";

export function useEnvironmentTargets() {
  return useQuery({
    queryKey: ["environment-targets"],
    queryFn: listEnvironmentTargets,
    staleTime: 60_000,
  });
}
