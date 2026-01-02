// Session and Message hooks (M2)

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  listSessions,
  updateSession,
  sendUserMessage,
  listEvents,
} from "@/lib/api/sessions";
import type { CreateSessionRequest, UpdateSessionRequest, Controls } from "@/lib/api/types";

export function useSessions(agentId: string | undefined) {
  return useQuery({
    queryKey: ["sessions", agentId],
    queryFn: () => listSessions(agentId!),
    enabled: !!agentId,
  });
}

export function useSession(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["session", agentId, sessionId],
    queryFn: () => getSession(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request?: CreateSessionRequest;
    }) => createSession(agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
    },
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: UpdateSessionRequest;
    }) => updateSession(agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
      queryClient.invalidateQueries({
        queryKey: ["session", agentId, sessionId],
      });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
    }: {
      agentId: string;
      sessionId: string;
    }) => deleteSession(agentId, sessionId),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
    },
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      content,
      controls,
    }: {
      agentId: string;
      sessionId: string;
      content: string;
      controls?: Controls;
    }) => sendUserMessage(agentId, sessionId, content, controls),
    onSuccess: (_, { agentId, sessionId }) => {
      // Invalidate events query to refresh the message list
      queryClient.invalidateQueries({
        queryKey: ["events", agentId, sessionId],
      });
    },
  });
}

// ============================================
// Events hook - returns raw events for direct rendering
// ============================================

/**
 * Fetch events for a session
 * Returns raw Event[] for direct rendering in the UI
 */
export function useEvents(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["events", agentId, sessionId],
    queryFn: () => listEvents(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}
