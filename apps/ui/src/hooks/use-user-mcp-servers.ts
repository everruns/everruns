"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  addUserMcpServer,
  getUserMcpServerTools,
  listUserMcpServers,
  removeUserMcpServer,
  setUserMcpToolLabel,
  updateUserMcpServer,
} from "@/lib/api/user-mcp-servers";
import type {
  AddUserMcpServerRequest,
  McpServerTool,
  McpToolLabel,
  UpdateUserMcpServerRequest,
} from "@/lib/api/types";
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

// Removing a server also signs the person out of it, so personal sign-ins
// refresh with the list.
function useInvalidate() {
  const queryClient = useQueryClient();
  return () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.userMcpServers.all }),
      queryClient.invalidateQueries({ queryKey: queryKeys.userConnections.all }),
    ]);
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

/** Tools of one of the person's custom servers; the server is asked live. */
export function useUserMcpServerTools(serverId?: string, identityId = "me") {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.userMcpServers.tools(org, identityId, serverId ?? ""),
    queryFn: () => getUserMcpServerTools(serverId!, identityId),
    enabled: !!org && !!serverId,
    staleTime: 0,
    retry: false,
  });
}

export function useSetUserMcpToolLabel(serverId: string, identityId = "me") {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const key = queryKeys.userMcpServers.tools(currentOrg?.public_id, identityId, serverId);
  return useMutation({
    mutationFn: ({ toolName, label }: { toolName: string; label: McpToolLabel | null }) =>
      setUserMcpToolLabel(serverId, toolName, label, identityId),
    // The answer carries only the label: the server is not asked again.
    onSuccess: (saved) => {
      queryClient.setQueryData<McpServerTool[]>(key, (tools) =>
        tools?.map((tool) => (tool.name === saved.name ? { ...tool, label: saved.label } : tool)),
      );
    },
  });
}
