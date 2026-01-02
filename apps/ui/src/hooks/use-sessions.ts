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
import type { CreateSessionRequest, UpdateSessionRequest, Event, Message, ContentPart, Controls } from "@/lib/api/types";
import { isToolResultPart } from "@/lib/api/types";

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
// Events hooks and helpers
// ============================================

/**
 * Extract tool_call_id from content parts (for tool_result messages)
 */
function extractToolCallId(content: ContentPart[]): string | null {
  for (const part of content) {
    if (isToolResultPart(part)) {
      return part.tool_call_id;
    }
  }
  return null;
}

/**
 * Transform events to Message-like objects for UI rendering
 * This allows the UI to render from events while still displaying as "messages"
 */
function eventsToMessages(events: Event[]): Message[] {
  // Filter message events and tool.call_completed events
  // Note: Tool calls are embedded in message.agent events via ContentPart::ToolCall
  // Note: Tool results now come from tool.call_completed events (not message.tool_result)
  const relevantEvents = events.filter(e =>
    e.type === "message.user" ||
    e.type === "message.agent" ||
    e.type === "tool.call_completed"
  );

  return relevantEvents.map((event, index) => {
    if (event.type === "tool.call_completed") {
      // Convert tool.call_completed to a tool_result message
      const data = event.data as {
        tool_call_id: string;
        tool_name: string;
        success: boolean;
        result?: ContentPart[];
        error?: string;
      };

      // Build content from result or error
      const content: ContentPart[] = data.result || [];
      if (data.error) {
        content.push({ type: "text", text: data.error } as ContentPart);
      }

      return {
        id: event.id,
        session_id: event.session_id,
        sequence: event.sequence ?? index,
        role: "tool_result" as Message["role"],
        content,
        metadata: undefined,
        tool_call_id: data.tool_call_id,
        created_at: event.ts,
      };
    }

    // Handle message.user and message.agent
    // data.message contains the full Message object
    const data = event.data as {
      message: {
        id: string;
        role: string;
        content: ContentPart[];
        created_at: string;
      };
    };

    const message = data.message;

    // Map event type to message role
    const roleMap: Record<string, Message["role"]> = {
      "message.user": "user",
      "message.agent": "assistant",
    };

    return {
      id: message?.id || event.id,
      session_id: event.session_id,
      sequence: event.sequence ?? index,
      role: roleMap[event.type] ?? message?.role as Message["role"],
      content: message?.content || [],
      metadata: undefined,
      tool_call_id: null,
      created_at: message?.created_at ?? event.ts,
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

/**
 * Fetch raw events without transformation for developer debugging
 * Returns all events including non-message events (step.*, tool.*, session.*)
 */
export function useRawEvents(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["raw-events", agentId, sessionId],
    queryFn: () => listEvents(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}
