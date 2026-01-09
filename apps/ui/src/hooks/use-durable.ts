// Durable Execution hooks

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getDurableHealth,
  listWorkers,
  drainWorker,
  listWorkflows,
  getWorkflow,
  getWorkflowEvents,
  cancelWorkflow,
  signalWorkflow,
  listTasks,
  getTaskStats,
  listDlq,
  requeueDlqEntry,
  deleteDlqEntry,
  purgeDlq,
  listCircuitBreakers,
  resetCircuitBreaker,
  type ListWorkflowsParams,
  type ListTasksParams,
  type ListDlqParams,
} from "@/lib/api/durable";

// ============================================
// System Health
// ============================================

export function useDurableHealth() {
  return useQuery({
    queryKey: ["durable", "health"],
    queryFn: getDurableHealth,
    refetchInterval: 5000, // Refresh every 5 seconds
  });
}

// ============================================
// Workers
// ============================================

export function useWorkers() {
  return useQuery({
    queryKey: ["durable", "workers"],
    queryFn: listWorkers,
    refetchInterval: 5000, // Refresh every 5 seconds
  });
}

export function useDrainWorker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (workerId: string) => drainWorker(workerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "workers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

// ============================================
// Workflows
// ============================================

export function useWorkflows(params?: ListWorkflowsParams) {
  return useQuery({
    queryKey: ["durable", "workflows", params],
    queryFn: () => listWorkflows(params),
    refetchInterval: 5000,
  });
}

export function useWorkflow(workflowId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "workflow", workflowId],
    queryFn: () => getWorkflow(workflowId!),
    enabled: !!workflowId,
    refetchInterval: 2000,
  });
}

export function useWorkflowEvents(workflowId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "workflow", workflowId, "events"],
    queryFn: () => getWorkflowEvents(workflowId!),
    enabled: !!workflowId,
    refetchInterval: 2000,
  });
}

export function useCancelWorkflow() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (workflowId: string) => cancelWorkflow(workflowId),
    onSuccess: (_, workflowId) => {
      queryClient.invalidateQueries({ queryKey: ["durable", "workflows"] });
      queryClient.invalidateQueries({
        queryKey: ["durable", "workflow", workflowId],
      });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

export function useSignalWorkflow() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      workflowId,
      signalType,
      payload,
    }: {
      workflowId: string;
      signalType: string;
      payload?: Record<string, unknown>;
    }) => signalWorkflow(workflowId, signalType, payload),
    onSuccess: (_, { workflowId }) => {
      queryClient.invalidateQueries({
        queryKey: ["durable", "workflow", workflowId],
      });
    },
  });
}

// ============================================
// Tasks
// ============================================

export function useTasks(params?: ListTasksParams) {
  return useQuery({
    queryKey: ["durable", "tasks", params],
    queryFn: () => listTasks(params),
    refetchInterval: 5000,
  });
}

export function useTaskStats() {
  return useQuery({
    queryKey: ["durable", "tasks", "stats"],
    queryFn: getTaskStats,
    refetchInterval: 5000,
  });
}

// ============================================
// Dead Letter Queue
// ============================================

export function useDlq(params?: ListDlqParams) {
  return useQuery({
    queryKey: ["durable", "dlq", params],
    queryFn: () => listDlq(params),
    refetchInterval: 10000, // DLQ changes less frequently
  });
}

export function useRequeueDlqEntry() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (dlqId: string) => requeueDlqEntry(dlqId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "dlq"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "tasks"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

export function useDeleteDlqEntry() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (dlqId: string) => deleteDlqEntry(dlqId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "dlq"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

export function usePurgeDlq() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: purgeDlq,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "dlq"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

// ============================================
// Circuit Breakers
// ============================================

export function useCircuitBreakers() {
  return useQuery({
    queryKey: ["durable", "circuit-breakers"],
    queryFn: listCircuitBreakers,
    refetchInterval: 10000,
  });
}

export function useResetCircuitBreaker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (key: string) => resetCircuitBreaker(key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}
