"use client";

import { useQuery } from "@tanstack/react-query";
import { getSessionCommands } from "@/lib/api/commands";
import { queryKeys } from "@/lib/query-keys";

export function useSessionCommands(sessionId: string) {
  return useQuery({
    queryKey: queryKeys.commands.list(sessionId),
    queryFn: () => getSessionCommands(sessionId),
    enabled: !!sessionId,
    staleTime: 60000,
  });
}
