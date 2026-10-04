"use client";

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { useOrg } from "@/providers/org-provider";
import type { Session } from "@/lib/api/types";

export const PLATFORM_CHAT_THREAD_TITLE = "Chat";
export interface UsePlatformChatThreadOptions {
  ensure?: boolean;
}
export interface UsePlatformChatThreadResult {
  thread?: Session;
  isLoading: boolean;
  error: Error | null;
}

/** The server resolves the durable starter directly and arbitrates concurrent creation. */
export function usePlatformChatThread(
  options: UsePlatformChatThreadOptions = {},
): UsePlatformChatThreadResult {
  const { currentOrg, isLoading } = useOrg();
  const query = useQuery({
    queryKey: ["sessions", "platform-chat", currentOrg?.public_id],
    queryFn: async () => (await api.post<Session>("/v1/sessions/platform-chat", {})).data,
    enabled: !!currentOrg && (options.ensure ?? true),
    staleTime: Infinity,
  });
  return { thread: query.data, isLoading: isLoading || query.isLoading, error: query.error };
}
