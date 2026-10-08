"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listChatMcpServers, removeChatMcpServer } from "@/lib/api/session-mcp-servers";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

/** MCP servers added to this chat only. */
export function useChatMcpServers(sessionId: string | undefined) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.chatMcpServers.list(sessionId ?? ""),
    queryFn: () => listChatMcpServers(sessionId!),
    enabled: !!org && !!sessionId,
  });
}

export function useRemoveChatMcpServer(sessionId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => removeChatMcpServer(sessionId, name),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.chatMcpServers.list(sessionId) }),
  });
}
