// Durable Execution hooks
// Uses SSE for real-time updates instead of polling

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import {
  getDurableHealth,
  listWorkers,
  drainWorker,
  resumeWorker,
  listWorkflows,
  getWorkflow,
  getWorkflowEvents,
  cancelWorkflow,
  signalWorkflow,
  listTasks,
  getTaskStats,
  enqueueTask,
  type EnqueueTaskRequest,
  listDlq,
  requeueDlqEntry,
  deleteDlqEntry,
  purgeDlq,
  listCircuitBreakers,
  forceOpenCircuitBreaker,
  forceCloseCircuitBreaker,
  deleteCircuitBreaker,
  getDurableSseUrl,
  getWorkflowSseUrl,
  getDurableMetrics,
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
  MetricsPoint,
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
  metrics_history: MetricsPoint[];
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
  const reconnectRef = useRef(createReconnectTracker());
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

    reconnectRef.current = createReconnectTracker();

    const connectSSE = () => {
      cleanup();

      const sseUrl = getDurableSseUrl();
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        reconnectRef.current.reset();
        setIsConnected(true);
        setError(null);
      });

      // Listen for "disconnecting" event for graceful connection cycling
      eventSource.addEventListener("disconnecting", (event) => {
        try {
          const data = JSON.parse(event.data);
          const retryMs = reconnectRef.current.onGraceful(data.retry_ms ?? 1000);
          console.debug("Durable SSE disconnecting, reconnecting in", retryMs, "ms");
          cleanup();
          setTimeout(() => {
            if (isEnabled) {
              connectSSE();
            }
          }, retryMs);
        } catch {
          cleanup();
          if (isEnabled) {
            connectSSE();
          }
        }
      });

      eventSource.addEventListener("snapshot", (event) => {
        try {
          const snapshot: DurableSnapshot = JSON.parse(event.data);

          // Update all durable query caches
          queryClient.setQueryData(["durable", "health"], snapshot.health);

          // Compute worker summary from the workers array
          const activeWorkers = snapshot.workers.filter((w) => w.status === "active");
          const workersSummary = {
            active: activeWorkers.length,
            draining: snapshot.workers.filter((w) => w.status === "draining").length,
            stopped: snapshot.workers.filter((w) => w.status === "stopped").length,
            total_capacity: activeWorkers.reduce((sum, w) => sum + w.max_concurrency, 0),
            total_load: activeWorkers.reduce((sum, w) => sum + w.current_load, 0),
          };

          queryClient.setQueryData(["durable", "workers"], {
            data: snapshot.workers,
            total: snapshot.workers.length,
            summary: workersSummary,
          });
          queryClient.setQueryData(["durable", "workflows", undefined], snapshot.workflows);
          queryClient.setQueryData(["durable", "tasks", undefined], snapshot.tasks);
          queryClient.setQueryData(["durable", "dlq", undefined], snapshot.dlq);
          queryClient.setQueryData(["durable", "circuit-breakers"], snapshot.circuit_breakers);
          if (snapshot.metrics_history) {
            queryClient.setQueryData(["durable", "metrics"], {
              points: snapshot.metrics_history,
              resolution_seconds: 10,
            });
          }
        } catch (e) {
          console.error("Failed to parse durable SSE snapshot:", e);
        }
      });

      eventSource.onerror = () => {
        setIsConnected(false);
        cleanup();
        const delayMs = reconnectRef.current.onError();
        if (delayMs === null) {
          setError(new Error("Durable SSE connection failed after max retries"));
          return;
        }
        setError(new Error("Durable SSE connection error, reconnecting..."));
        setTimeout(() => {
          if (isEnabled) {
            connectSSE();
          }
        }, delayMs);
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
export function useWorkflowSSE(workflowId: string | undefined, options?: { enabled?: boolean }) {
  const queryClient = useQueryClient();
  const eventSourceRef = useRef<EventSource | null>(null);
  const reconnectRef = useRef(createReconnectTracker());
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

    reconnectRef.current = createReconnectTracker();

    const connectSSE = () => {
      cleanup();

      const sseUrl = getWorkflowSseUrl(workflowId);
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        reconnectRef.current.reset();
        setIsConnected(true);
        setError(null);
      });

      // Listen for "disconnecting" event for graceful connection cycling
      eventSource.addEventListener("disconnecting", (event) => {
        try {
          const data = JSON.parse(event.data);
          const retryMs = reconnectRef.current.onGraceful(data.retry_ms ?? 1000);
          console.debug("Workflow SSE disconnecting, reconnecting in", retryMs, "ms");
          cleanup();
          setTimeout(() => {
            if (isEnabled && workflowId) {
              connectSSE();
            }
          }, retryMs);
        } catch {
          cleanup();
          if (isEnabled && workflowId) {
            connectSSE();
          }
        }
      });

      eventSource.addEventListener("snapshot", (event) => {
        try {
          const snapshot: WorkflowSnapshot = JSON.parse(event.data);

          // Update workflow and events caches
          queryClient.setQueryData(["durable", "workflow", workflowId], snapshot.workflow);
          queryClient.setQueryData(["durable", "workflow", workflowId, "events"], snapshot.events);
        } catch (e) {
          console.error("Failed to parse workflow SSE snapshot:", e);
        }
      });

      eventSource.onerror = () => {
        setIsConnected(false);
        cleanup();
        const delayMs = reconnectRef.current.onError();
        if (delayMs === null) {
          setError(new Error("Workflow SSE connection failed after max retries"));
          return;
        }
        setError(new Error("Workflow SSE connection error, reconnecting..."));
        setTimeout(() => {
          if (isEnabled && workflowId) {
            connectSSE();
          }
        }, delayMs);
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

export function useResumeWorker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (workerId: string) => resumeWorker(workerId),
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

export function useEnqueueTask() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: EnqueueTaskRequest) => enqueueTask(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "tasks"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

// ============================================
// Metrics Time Series (SSE-backed + REST fallback)
// ============================================

export function useDurableMetrics() {
  return useQuery({
    queryKey: ["durable", "metrics"],
    queryFn: getDurableMetrics,
    staleTime: Infinity, // SSE provides updates
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

export function useForceOpenCircuitBreaker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (key: string) => forceOpenCircuitBreaker(key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

export function useForceCloseCircuitBreaker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (key: string) => forceCloseCircuitBreaker(key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

export function useDeleteCircuitBreaker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (key: string) => deleteCircuitBreaker(key),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "circuit-breakers"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "health"] });
    },
  });
}

// ============================================
// Schedules
// ============================================

import {
  listSchedules,
  getSchedule,
  createSchedule,
  updateSchedule,
  deleteSchedule as deleteScheduleApi,
  pauseSchedule,
  resumeSchedule,
  triggerSchedule,
  listScheduleExecutions,
  getScheduleStats,
  type ListSchedulesParams,
  type ListExecutionsParams,
} from "@/lib/api/durable";
import type { CreateScheduleRequest, UpdateScheduleRequest } from "@/lib/api/types";

export function useSchedules(params?: ListSchedulesParams) {
  return useQuery({
    queryKey: ["durable", "schedules", params],
    queryFn: () => listSchedules(params),
    staleTime: 30000, // 30 seconds - schedules don't change as often
  });
}

export function useSchedule(scheduleId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "schedule", scheduleId],
    queryFn: () => getSchedule(scheduleId!),
    enabled: !!scheduleId,
    staleTime: 30000,
  });
}

export function useCreateSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateScheduleRequest) => createSchedule(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
    },
  });
}

export function useUpdateSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ scheduleId, request }: { scheduleId: string; request: UpdateScheduleRequest }) =>
      updateSchedule(scheduleId, request),
    onSuccess: (_, { scheduleId }) => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "schedule", scheduleId] });
    },
  });
}

export function useDeleteSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (scheduleId: string) => deleteScheduleApi(scheduleId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
    },
  });
}

export function usePauseSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (scheduleId: string) => pauseSchedule(scheduleId),
    onSuccess: (_, scheduleId) => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "schedule", scheduleId] });
    },
  });
}

export function useResumeSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (scheduleId: string) => resumeSchedule(scheduleId),
    onSuccess: (_, scheduleId) => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "schedule", scheduleId] });
    },
  });
}

export function useTriggerSchedule() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (scheduleId: string) => triggerSchedule(scheduleId),
    onSuccess: (_, scheduleId) => {
      queryClient.invalidateQueries({ queryKey: ["durable", "schedules"] });
      queryClient.invalidateQueries({ queryKey: ["durable", "schedule", scheduleId] });
      queryClient.invalidateQueries({
        queryKey: ["durable", "schedule", scheduleId, "executions"],
      });
    },
  });
}

export function useScheduleExecutions(
  scheduleId: string | undefined,
  params?: ListExecutionsParams,
) {
  return useQuery({
    queryKey: ["durable", "schedule", scheduleId, "executions", params],
    queryFn: () => listScheduleExecutions(scheduleId!, params),
    enabled: !!scheduleId,
    staleTime: 10000, // 10 seconds - executions update more frequently
  });
}

export function useScheduleStats(scheduleId: string | undefined) {
  return useQuery({
    queryKey: ["durable", "schedule", scheduleId, "stats"],
    queryFn: () => getScheduleStats(scheduleId!),
    enabled: !!scheduleId,
    staleTime: 30000,
  });
}
