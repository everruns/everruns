"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getConnectedClients, revokeConnectedClient } from "@/lib/api/connected-clients";
import { queryKeys } from "@/lib/query-keys";

export function useConnectedClients() {
  return useQuery({
    queryKey: queryKeys.connectedClients.list(),
    queryFn: async () => (await getConnectedClients()).data,
    staleTime: 30000,
  });
}

export function useRevokeConnectedClient() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (grantId: string) => revokeConnectedClient(grantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.connectedClients.all });
    },
  });
}
