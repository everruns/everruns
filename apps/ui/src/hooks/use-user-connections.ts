"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getUserConnections,
  deleteUserConnection,
  putApiKeyConnection,
} from "@/lib/api/user-connections";
import { queryKeys } from "@/lib/query-keys";

export function useUserConnections() {
  return useQuery({
    queryKey: queryKeys.userConnections.list(),
    queryFn: () => getUserConnections(),
    staleTime: 30000,
  });
}

export function useDeleteUserConnection() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (provider: string) => deleteUserConnection(provider),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.userConnections.all });
    },
  });
}

export function usePutApiKeyConnection() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ provider, apiKey }: { provider: string; apiKey: string }) =>
      putApiKeyConnection(provider, apiKey),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.userConnections.all });
    },
  });
}
