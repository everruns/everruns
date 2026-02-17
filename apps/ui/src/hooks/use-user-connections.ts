"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getUserConnections,
  getConnectionProviders,
  createApiKeyConnection,
  deleteUserConnection,
} from "@/lib/api/user-connections";
import { queryKeys } from "@/lib/query-keys";

export function useUserConnections() {
  return useQuery({
    queryKey: queryKeys.userConnections.list(),
    queryFn: () => getUserConnections(),
    staleTime: 30000,
  });
}

export function useConnectionProviders() {
  return useQuery({
    queryKey: ["connection-providers"],
    queryFn: () => getConnectionProviders(),
    staleTime: 60000,
  });
}

export function useCreateApiKeyConnection() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ provider, apiKey }: { provider: string; apiKey: string }) =>
      createApiKeyConnection(provider, apiKey),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.userConnections.all,
      });
    },
  });
}

export function useDeleteUserConnection() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (provider: string) => deleteUserConnection(provider),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.userConnections.all,
      });
    },
  });
}
