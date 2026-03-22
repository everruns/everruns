"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  listIdentityConnections,
  createIdentityApiKeyConnection,
  deleteIdentityConnection,
  verifyIdentityConnection,
} from "@/lib/api/agent-identity-connections";
import { queryKeys } from "@/lib/query-keys";

export function useIdentityConnections(identityId: string) {
  return useQuery({
    queryKey: queryKeys.identityConnections.list(identityId),
    queryFn: () => listIdentityConnections(identityId),
    staleTime: 30000,
  });
}

export function useCreateIdentityApiKeyConnection(identityId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ provider, apiKey }: { provider: string; apiKey: string }) =>
      createIdentityApiKeyConnection(identityId, provider, apiKey),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.identityConnections.list(identityId),
      });
    },
  });
}

export function useDeleteIdentityConnection(identityId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (provider: string) => deleteIdentityConnection(identityId, provider),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.identityConnections.list(identityId),
      });
    },
  });
}

export function useVerifyIdentityConnection(identityId: string) {
  return useMutation({
    mutationFn: (provider: string) => verifyIdentityConnection(identityId, provider),
  });
}
