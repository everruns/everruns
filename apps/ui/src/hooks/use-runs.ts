"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { getRuns, getRun, createRun, cancelRun } from "@/lib/api/runs";
import type { CreateRunRequest } from "@/lib/api/types";

export function useRuns(params?: {
  status?: string;
  agent_id?: string;
  limit?: number;
  offset?: number;
}) {
  return useQuery({
    queryKey: ["runs", params],
    queryFn: () => getRuns(params),
    staleTime: 10000, // 10 seconds
    refetchInterval: 10000, // Auto-refresh every 10 seconds
  });
}

export function useRun(runId: string) {
  return useQuery({
    queryKey: ["runs", runId],
    queryFn: () => getRun(runId),
    enabled: !!runId,
    staleTime: 5000, // 5 seconds
  });
}

export function useCreateRun() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateRunRequest) => createRun(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["runs"] });
    },
  });
}

export function useCancelRun() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (runId: string) => cancelRun(runId),
    onSuccess: (_, runId) => {
      queryClient.invalidateQueries({ queryKey: ["runs"] });
      queryClient.invalidateQueries({ queryKey: ["runs", runId] });
    },
  });
}
