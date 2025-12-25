// Session and Message hooks (M2)

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  listSessions,
  updateSession,
  sendUserMessage,
  listMessages,
  listEvents,
} from "@/lib/api/sessions";
import type { CreateSessionRequest, UpdateSessionRequest, Event, Message, ContentPart } from "@/lib/api/types";

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
    }: {
      agentId: string;
      sessionId: string;
      content: string;
    }) => sendUserMessage(agentId, sessionId, content),
    onSuccess: (_, { agentId, sessionId }) => {
      // Invalidate both messages and events queries for backward compatibility
      queryClient.invalidateQueries({
        queryKey: ["messages", agentId, sessionId],
      });
      queryClient.invalidateQueries({
        queryKey: ["events", agentId, sessionId],
      });
    },
  });
}

export function useMessages(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["messages", agentId, sessionId],
    queryFn: () => listMessages(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}

// ============================================
// Events hooks and helpers
// ============================================

/**
 * Transform events to Message-like objects for UI rendering
 * This allows the UI to render from events while still displaying as "messages"
 */
function eventsToMessages(events: Event[]): Message[] {
  // Filter only message events and transform them
  const messageEvents = events.filter(e =>
    e.event_type === "message.user" ||
    e.event_type === "message.assistant" ||
    e.event_type === "message.tool_call" ||
    e.event_type === "message.tool_result"
  );

  return messageEvents.map(event => {
    const data = event.data as {
      message_id: string;
      role: string;
      content: ContentPart[];
      sequence: number;
      created_at: string;
    };

    // Map event type to message role
    const roleMap: Record<string, Message["role"]> = {
      "message.user": "user",
      "message.assistant": "assistant",
      "message.tool_call": "tool_call",
      "message.tool_result": "tool_result",
    };

    return {
      id: data.message_id,
      session_id: event.session_id,
      sequence: data.sequence ?? event.sequence,
      role: roleMap[event.event_type] ?? data.role as Message["role"],
      content: data.content,
      metadata: undefined,
      tool_call_id: null, // Derived from content if needed
      created_at: data.created_at ?? event.created_at,
    };
  });
}

/**
 * Fetch events and transform them to messages for UI rendering
 * This hook replaces useMessages for event-based rendering
 */
export function useEvents(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["events", agentId, sessionId],
    queryFn: async () => {
      const events = await listEvents(agentId!, sessionId!);
      return eventsToMessages(events);
    },
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}
