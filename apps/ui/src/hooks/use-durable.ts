// Durable Execution hooks
// Uses SSE for real-time updates instead of polling

import { useCallback, useEffect, useRef, useState } from "react";
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
  getDurableSseUrl,
  getWorkflowSseUrl,
  type ListWorkflowsParams,
  type ListTasksParams,
  type ListDlqParams,
} from "@/lib/api/durable";
import type {
  DurableSystemHealth,
  DurableWorker,
  DurableWorkflow,
  WorkflowEvent,
  DurableTask,
  DlqEntry,
  CircuitBreaker,
} from "@/lib/api/types";

// ============================================
// SSE Snapshot Types (match backend response)
// ============================================

interface DurableSnapshot {
  health: DurableSystemHealth;
  workers: DurableWorker[];
  workflows: { data: DurableWorkflow[]; total: number };
  tasks: { data: DurableTask[]; total: number };
  dlq: { data: DlqEntry[]; total: number };
  circuit_breakers: { data: CircuitBreaker[]; total: number };
}

interface WorkflowSnapshot {
  workflow: DurableWorkflow;
  events: WorkflowEvent[];
}

// ============================================
// Global Durable SSE Hook
// ============================================

/**
 * Connect to global durable SSE stream and update React Query cache.
 * This single connection provides real-time updates for all durable data.
 */
export function useDurableSSE(options?: { enabled?: boolean }) {
  const queryClient = useQueryClient();
  const eventSourceRef = useRef<EventSource | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const isEnabled = options?.enabled !== false;

  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!isEnabled) {
      cleanup();
      setIsConnected(false);
      return;
    }

    const connectSSE = () => {
      cleanup();

      const sseUrl = getDurableSseUrl();
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        setIsConnected(true);
        setError(null);
      });

      eventSource.addEventListener("snapshot", (event) => {
        try {
          const snapshot: DurableSnapshot = JSON.parse(event.data);

          // Update all durable query caches
          queryClient.setQueryData(["durable", "health"], snapshot.health);
          queryClient.setQueryData(["durable", "workers"], {
            data: snapshot.workers,
            total: snapshot.workers.length,
          });
          queryClient.setQueryData(["durable", "workflows", undefined], snapshot.workflows);
          queryClient.setQueryData(["durable", "tasks", undefined], snapshot.tasks);
          queryClient.setQueryData(["durable", "dlq", undefined], snapshot.dlq);
          queryClient.setQueryData(
            ["durable", "circuit-breakers"],
            snapshot.circuit_breakers
          );
        } catch (e) {
          console.error("Failed to parse durable SSE snapshot:", e);
        }
      });

      eventSource.onerror = () => {
        setError(new Error("Durable SSE connection error"));
        setIsConnected(false);
        cleanup();
        // Reconnect after delay
        setTimeout(() => {
          if (isEnabled) {
            connectSSE();
          }
        }, 2000);
      };
    };

    connectSSE();

    return cleanup;
  }, [isEnabled, cleanup, queryClient]);

  return { isConnected, error };
}

// ============================================
// Per-Workflow SSE Hook
// ============================================

/**
 * Connect to workflow-specific SSE stream for real-time workflow updates.
 * Updates the specific workflow and its events in React Query cache.
 */
export function useWorkflowSSE(
  workflowId: string | undefined,
  options?: { enabled?: boolean }
) {
  const queryClient = useQueryClient();
  const eventSourceRef = useRef<EventSource | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const isEnabled = options?.enabled !== false && !!workflowId;

  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  // Reset state when workflow changes
  useEffect(() => {
    setIsConnected(false);
    setError(null);
  }, [workflowId]);

  useEffect(() => {
    if (!isEnabled || !workflowId) {
      cleanup();
      setIsConnected(false);
      return;
    }

    const connectSSE = () => {
      cleanup();

      const sseUrl = getWorkflowSseUrl(workflowId);
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        setIsConnected(true);
        setError(null);
      });

      eventSource.addEventListener("snapshot", (event) => {
        try {
          const snapshot: WorkflowSnapshot = JSON.parse(event.data);

          // Update workflow and events caches
          queryClient.setQueryData(
            ["durable", "workflow", workflowId],
            snapshot.workflow
          );
          queryClient.setQueryData(
            ["durable", "workflow", workflowId, "events"],
            snapshot.events
          );
        } catch (e) {
          console.error("Failed to parse workflow SSE snapshot:", e);
        }
      });

      eventSource.onerror = () => {
        setError(new Error("Workflow SSE connection error"));
        setIsConnected(false);
        cleanup();
        // Reconnect after delay
        setTimeout(() => {
          if (isEnabled && workflowId) {
            connectSSE();
          }
        }, 2000);
      };
    };

    connectSSE();

    return cleanup;
  }, [workflowId, isEnabled, cleanup, queryClient]);

  return { isConnected, error };
}

// ============================================
// System Health (SSE-backed)
// ============================================

export function useDurableHealth() {
  return useQuery({
    queryKey: ["durable", "health"],
    queryFn: getDurableHealth,
    // No refetchInterval - SSE provides updates
    staleTime: Infinity, // Data is always fresh from SSE
  });
}

// ============================================
// Workers (SSE-backed)
// ============================================

export function useWorkers() {
  return useQuery({
    queryKey: ["durable", "workers"],
    queryFn: listWorkers,
    staleTime: Infinity,
  });
}

export function useDrainWorker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (workerId: string) => drainWorker(workerId),
    onSuccess: () => {
      // Invalidate to trigger immediate refetch; SSE will update shortly
      queryClient.invalidateQueries({ queryKey: ["durable", "workers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

// ============================================
// Workflows (SSE-backed)
// ============================================

export function useWorkflows(params?: ListWorkflowsParams) {
  return useQuery({
    queryKey: ["durable", "workflows", params],
    queryFn: () => listWorkflows(params),
    staleTime: Infinity,
  });
}

export function useWorkflow(workflowId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "workflow", workflowId],
    queryFn: () => getWorkflow(workflowId!),
    enabled: !!workflowId,
    staleTime: Infinity,
  });
}

export function useWorkflowEvents(workflowId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "workflow", workflowId, "events"],
    queryFn: () => getWorkflowEvents(workflowId!),
    enabled: !!workflowId,
    staleTime: Infinity,
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
// Tasks (SSE-backed)
// ============================================

export function useTasks(params?: ListTasksParams) {
  return useQuery({
    queryKey: ["durable", "tasks", params],
    queryFn: () => listTasks(params),
    staleTime: Infinity,
  });
}

export function useTaskStats() {
  return useQuery({
    queryKey: ["durable", "tasks", "stats"],
    queryFn: getTaskStats,
    staleTime: Infinity,
  });
}

// ============================================
// Dead Letter Queue (SSE-backed)
// ============================================

export function useDlq(params?: ListDlqParams) {
  return useQuery({
    queryKey: ["durable", "dlq", params],
    queryFn: () => listDlq(params),
    staleTime: Infinity,
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
// Circuit Breakers (SSE-backed)
// ============================================

export function useCircuitBreakers() {
  return useQuery({
    queryKey: ["durable", "circuit-breakers"],
    queryFn: listCircuitBreakers,
    staleTime: Infinity,
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
