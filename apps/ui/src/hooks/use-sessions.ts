// Session hooks (M2)

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  listSessions,
  updateSession,
  sendUserMessage,
  listMessages,
} from "@/lib/api/sessions";
import type { CreateSessionRequest, UpdateSessionRequest } from "@/lib/api/types";

export function useSessions(harnessId: string | undefined) {
  return useQuery({
    queryKey: ["sessions", harnessId],
    queryFn: () => listSessions(harnessId!),
    enabled: !!harnessId,
  });
}

export function useSession(
  harnessId: string | undefined,
  sessionId: string | undefined
) {
  return useQuery({
    queryKey: ["session", harnessId, sessionId],
    queryFn: () => getSession(harnessId!, sessionId!),
    enabled: !!harnessId && !!sessionId,
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      harnessId,
      request,
    }: {
      harnessId: string;
      request?: CreateSessionRequest;
    }) => createSession(harnessId, request),
    onSuccess: (_, { harnessId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", harnessId] });
    },
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      harnessId,
      sessionId,
      request,
    }: {
      harnessId: string;
      sessionId: string;
      request: UpdateSessionRequest;
    }) => updateSession(harnessId, sessionId, request),
    onSuccess: (_, { harnessId, sessionId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", harnessId] });
      queryClient.invalidateQueries({
        queryKey: ["session", harnessId, sessionId],
      });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      harnessId,
      sessionId,
    }: {
      harnessId: string;
      sessionId: string;
    }) => deleteSession(harnessId, sessionId),
    onSuccess: (_, { harnessId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", harnessId] });
    },
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      harnessId,
      sessionId,
      content,
    }: {
      harnessId: string;
      sessionId: string;
      content: string;
    }) => sendUserMessage(harnessId, sessionId, content),
    onSuccess: (_, { harnessId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: ["messages", harnessId, sessionId],
      });
    },
  });
}

export function useMessages(
  harnessId: string | undefined,
  sessionId: string | undefined
) {
  return useQuery({
    queryKey: ["messages", harnessId, sessionId],
    queryFn: () => listMessages(harnessId!, sessionId!),
    enabled: !!harnessId && !!sessionId,
  });
}
