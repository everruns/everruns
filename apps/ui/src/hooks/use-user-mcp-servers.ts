"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  addUserMcpServer,
  listUserMcpServers,
  removeUserMcpServer,
  updateUserMcpServer,
} from "@/lib/api/user-mcp-servers";
import type { AddUserMcpServerRequest, UpdateUserMcpServerRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

export function useUserMcpServers(identityId = "me") {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.userMcpServers.list(org, identityId),
    queryFn: () => listUserMcpServers(identityId),
    enabled: !!org,
    staleTime: 30000,
  });
}

function useInvalidate() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: queryKeys.userMcpServers.all });
}

export function useAddUserMcpServer(identityId = "me") {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (request: AddUserMcpServerRequest) => addUserMcpServer(request, identityId),
    onSuccess: invalidate,
  });
}

export function useUpdateUserMcpServer(identityId = "me") {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: ({
      serverId,
      request,
    }: {
      serverId: string;
      request: UpdateUserMcpServerRequest;
    }) => updateUserMcpServer(serverId, request, identityId),
    onSuccess: invalidate,
  });
}

export function useRemoveUserMcpServer(identityId = "me") {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (serverId: string) => removeUserMcpServer(serverId, identityId),
    onSuccess: invalidate,
  });
}
