"use client";
import { useQuery } from "@tanstack/react-query";
import { getChatGptConnection } from "@/lib/api/chatgpt";
import { useOrg } from "@/providers/org-provider";

export function useChatGptConnection(id: string, enabled = true) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["providers", "chatgpt", id, currentOrg?.public_id],
    queryFn: () => getChatGptConnection(id),
    enabled: enabled && !!currentOrg,
    refetchInterval: (query) => (query.state.data?.status === "connecting" ? 1500 : false),
  });
}
