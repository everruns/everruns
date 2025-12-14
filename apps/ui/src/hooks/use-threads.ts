"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createThread,
  getThread,
  getMessages,
  createMessage,
} from "@/lib/api/threads";
import type { CreateMessageRequest } from "@/lib/api/types";

export function useThread(threadId: string) {
  return useQuery({
    queryKey: ["threads", threadId],
    queryFn: () => getThread(threadId),
    enabled: !!threadId,
  });
}

export function useCreateThread() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => createThread(),
    onSuccess: (thread) => {
      queryClient.setQueryData(["threads", thread.id], thread);
    },
  });
}

export function useMessages(threadId: string) {
  return useQuery({
    queryKey: ["threads", threadId, "messages"],
    queryFn: () => getMessages(threadId),
    enabled: !!threadId,
    staleTime: 30000, // 30 seconds
  });
}

export function useCreateMessage(threadId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateMessageRequest) => createMessage(threadId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["threads", threadId, "messages"],
      });
    },
  });
}
