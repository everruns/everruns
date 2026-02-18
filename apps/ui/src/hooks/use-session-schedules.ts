// Session Schedule hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  listSessionSchedules,
  updateSessionSchedule,
  deleteSessionSchedule,
  triggerSessionSchedule,
} from "@/lib/api/session-schedules";
import type { UpdateSessionScheduleRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

/** List schedules for a session */
export function useSessionSchedules(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessionSchedules.list(sessionId!),
    queryFn: () => listSessionSchedules(sessionId!),
    enabled: !!org && !!sessionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Update a schedule (enable/disable) */
export function useUpdateSessionSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      sessionId,
      scheduleId,
      request,
    }: {
      sessionId: string;
      scheduleId: string;
      request: UpdateSessionScheduleRequest;
    }) => updateSessionSchedule(sessionId, scheduleId, request),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionSchedules.list(sessionId),
      });
    },
  });
}

/** Delete a schedule */
export function useDeleteSessionSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ sessionId, scheduleId }: { sessionId: string; scheduleId: string }) =>
      deleteSessionSchedule(sessionId, scheduleId),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionSchedules.list(sessionId),
      });
    },
  });
}

/** Manually trigger a schedule */
export function useTriggerSessionSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ sessionId, scheduleId }: { sessionId: string; scheduleId: string }) =>
      triggerSessionSchedule(sessionId, scheduleId),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionSchedules.list(sessionId),
      });
    },
  });
}
