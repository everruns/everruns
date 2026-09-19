"use client";

import { useInfiniteQuery, useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getUserConnections,
  getConnectionProviders,
  createApiKeyConnection,
  deleteUserConnection,
  getUserMcpConnections,
  verifyConnection,
} from "@/lib/api/user-connections";
import { queryKeys } from "@/lib/query-keys";

export function useUserConnections() {
  return useQuery({
    queryKey: queryKeys.userConnections.list(),
    queryFn: () => getUserConnections(),
    staleTime: 30000,
  });
}

export function useUserMcpConnections() {
  const query = useInfiniteQuery({
    queryKey: queryKeys.userConnections.mcp(),
    queryFn: ({ pageParam }) => getUserMcpConnections(pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    staleTime: 30000,
  });
  return {
    ...query,
    data: query.data?.pages.flatMap((page) => page.data),
  };
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
    mutationFn: ({
      provider,
      apiKey,
      extraFields,
    }: {
      provider: string;
      apiKey: string;
      extraFields?: Record<string, string>;
    }) => createApiKeyConnection(provider, apiKey, extraFields),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.userConnections.all,
      });
    },
  });
}

export function useVerifyConnection() {
  return useMutation({
    mutationFn: (provider: string) => verifyConnection(provider),
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
