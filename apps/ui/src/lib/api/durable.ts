// Durable Execution API functions

import { api, getApiBaseUrl } from "./client";
import type {
  WorkersResponse,
  WorkflowsResponse,
  DurableWorkflow,
  WorkflowEvent,
  WorkflowEventsResponse,
  TasksResponse,
  TaskQueueStats,
  DlqResponse,
  CircuitBreakersResponse,
  DurableSystemHealth,
} from "./types";

// ============================================
// SSE URLs
// ============================================

/**
 * Get SSE URL for global durable state streaming.
 * Returns health, workers, workflows, tasks, DLQ, and circuit breakers.
 */
export function getDurableSseUrl(): string {
  return `${getApiBaseUrl()}/v1/durable/sse`;
}

/**
 * Get SSE URL for workflow-specific state streaming.
 * Returns workflow details and events.
 */
export function getWorkflowSseUrl(workflowId: string): string {
  return `${getApiBaseUrl()}/v1/durable/workflows/${workflowId}/sse`;
}

// ============================================
// System Health
// ============================================

export async function getDurableHealth(): Promise<DurableSystemHealth> {
  const response = await api.get<DurableSystemHealth>("/v1/durable/health");
  return response.data;
}

// ============================================
// Workers
// ============================================

export async function listWorkers(): Promise<WorkersResponse> {
  const response = await api.get<WorkersResponse>("/v1/durable/workers");
  return response.data;
}

export async function drainWorker(workerId: string): Promise<void> {
  await api.post(`/v1/durable/workers/${encodeURIComponent(workerId)}/drain`);
}

export async function resumeWorker(workerId: string): Promise<void> {
  await api.post(`/v1/durable/workers/${encodeURIComponent(workerId)}/resume`);
}

// ============================================
// Workflows
// ============================================

export interface ListWorkflowsParams {
  status?: string;
  workflow_type?: string;
  limit?: number;
  offset?: number;
}

export async function listWorkflows(params?: ListWorkflowsParams): Promise<WorkflowsResponse> {
  const searchParams = new URLSearchParams();
  if (params?.status) searchParams.set("status", params.status);
  if (params?.workflow_type) searchParams.set("workflow_type", params.workflow_type);
  if (params?.limit) searchParams.set("limit", String(params.limit));
  if (params?.offset) searchParams.set("offset", String(params.offset));

  const query = searchParams.toString();
  const url = `/v1/durable/workflows${query ? `?${query}` : ""}`;
  const response = await api.get<WorkflowsResponse>(url);
  return response.data;
}

export async function getWorkflow(workflowId: string): Promise<DurableWorkflow> {
  const response = await api.get<DurableWorkflow>(`/v1/durable/workflows/${workflowId}`);
  return response.data;
}

export async function getWorkflowEvents(workflowId: string): Promise<WorkflowEvent[]> {
  const response = await api.get<WorkflowEventsResponse>(
    `/v1/durable/workflows/${workflowId}/events`,
  );
  return response.data.data;
}

export async function cancelWorkflow(workflowId: string): Promise<void> {
  await api.post(`/v1/durable/workflows/${workflowId}/cancel`);
}

export async function signalWorkflow(
  workflowId: string,
  signalType: string,
  payload?: Record<string, unknown>,
): Promise<void> {
  await api.post(`/v1/durable/workflows/${workflowId}/signal`, {
    signal_type: signalType,
    payload: payload || {},
  });
}

// ============================================
// Tasks
// ============================================

export interface ListTasksParams {
  status?: string;
  activity_type?: string;
  limit?: number;
  offset?: number;
}

export async function listTasks(params?: ListTasksParams): Promise<TasksResponse> {
  const searchParams = new URLSearchParams();
  if (params?.status) searchParams.set("status", params.status);
  if (params?.activity_type) searchParams.set("activity_type", params.activity_type);
  if (params?.limit) searchParams.set("limit", String(params.limit));
  if (params?.offset) searchParams.set("offset", String(params.offset));

  const query = searchParams.toString();
  const url = `/v1/durable/tasks${query ? `?${query}` : ""}`;
  const response = await api.get<TasksResponse>(url);
  return response.data;
}

export async function getTaskStats(): Promise<TaskQueueStats> {
  const response = await api.get<TaskQueueStats>("/v1/durable/tasks/stats");
  return response.data;
}

// ============================================
// Dead Letter Queue
// ============================================

export interface ListDlqParams {
  activity_type?: string;
  limit?: number;
  offset?: number;
}

export async function listDlq(params?: ListDlqParams): Promise<DlqResponse> {
  const searchParams = new URLSearchParams();
  if (params?.activity_type) searchParams.set("activity_type", params.activity_type);
  if (params?.limit) searchParams.set("limit", String(params.limit));
  if (params?.offset) searchParams.set("offset", String(params.offset));

  const query = searchParams.toString();
  const url = `/v1/durable/dlq${query ? `?${query}` : ""}`;
  const response = await api.get<DlqResponse>(url);
  return response.data;
}

export async function requeueDlqEntry(dlqId: string): Promise<string> {
  const response = await api.post<{ task_id: string }>(`/v1/durable/dlq/${dlqId}/requeue`);
  return response.data.task_id;
}

export async function deleteDlqEntry(dlqId: string): Promise<void> {
  await api.delete(`/v1/durable/dlq/${dlqId}`);
}

export async function purgeDlq(): Promise<{ deleted: number }> {
  const response = await api.post<{ deleted: number }>("/v1/durable/dlq/purge");
  return response.data;
}

// ============================================
// Circuit Breakers
// ============================================

export async function listCircuitBreakers(): Promise<CircuitBreakersResponse> {
  const response = await api.get<CircuitBreakersResponse>("/v1/durable/circuit-breakers");
  return response.data;
}

/**
 * Force open a circuit breaker.
 * Immediately blocks all calls through this circuit.
 */
export async function forceOpenCircuitBreaker(key: string): Promise<void> {
  await api.post(`/v1/durable/circuit-breakers/${encodeURIComponent(key)}/open`);
}

/**
 * Force close a circuit breaker.
 * Immediately allows calls through this circuit.
 */
export async function forceCloseCircuitBreaker(key: string): Promise<void> {
  await api.post(`/v1/durable/circuit-breakers/${encodeURIComponent(key)}/close`);
}

/**
 * Delete a circuit breaker (reset to default state).
 */
export async function deleteCircuitBreaker(key: string): Promise<void> {
  await api.delete(`/v1/durable/circuit-breakers/${encodeURIComponent(key)}`);
}

// ============================================
// Schedules
// ============================================

import type {
  DurableSchedule,
  SchedulesResponse,
  CreateScheduleRequest,
  UpdateScheduleRequest,
  ScheduleExecution,
  ScheduleExecutionsResponse,
  ScheduleStats,
  TriggerResponse,
} from "./types";

export interface ListSchedulesParams {
  enabled?: boolean;
  target_type?: string;
  limit?: number;
  offset?: number;
}

export async function listSchedules(params?: ListSchedulesParams): Promise<SchedulesResponse> {
  const searchParams = new URLSearchParams();
  if (params?.enabled !== undefined) searchParams.set("enabled", String(params.enabled));
  if (params?.target_type) searchParams.set("target_type", params.target_type);
  if (params?.limit) searchParams.set("limit", String(params.limit));
  if (params?.offset) searchParams.set("offset", String(params.offset));

  const query = searchParams.toString();
  const url = `/v1/durable/schedules${query ? `?${query}` : ""}`;
  const response = await api.get<SchedulesResponse>(url);
  return response.data;
}

export async function getSchedule(scheduleId: string): Promise<DurableSchedule> {
  const response = await api.get<DurableSchedule>(`/v1/durable/schedules/${scheduleId}`);
  return response.data;
}

export async function createSchedule(request: CreateScheduleRequest): Promise<DurableSchedule> {
  const response = await api.post<DurableSchedule>("/v1/durable/schedules", request);
  return response.data;
}

export async function updateSchedule(
  scheduleId: string,
  request: UpdateScheduleRequest,
): Promise<DurableSchedule> {
  const response = await api.patch<DurableSchedule>(`/v1/durable/schedules/${scheduleId}`, request);
  return response.data;
}

export async function deleteSchedule(scheduleId: string): Promise<void> {
  await api.delete(`/v1/durable/schedules/${scheduleId}`);
}

export async function pauseSchedule(scheduleId: string): Promise<DurableSchedule> {
  const response = await api.post<DurableSchedule>(`/v1/durable/schedules/${scheduleId}/pause`);
  return response.data;
}

export async function resumeSchedule(scheduleId: string): Promise<DurableSchedule> {
  const response = await api.post<DurableSchedule>(`/v1/durable/schedules/${scheduleId}/resume`);
  return response.data;
}

export async function triggerSchedule(scheduleId: string): Promise<TriggerResponse> {
  const response = await api.post<TriggerResponse>(`/v1/durable/schedules/${scheduleId}/trigger`);
  return response.data;
}

export interface ListExecutionsParams {
  status?: string;
  limit?: number;
  offset?: number;
}

export async function listScheduleExecutions(
  scheduleId: string,
  params?: ListExecutionsParams,
): Promise<ScheduleExecutionsResponse> {
  const searchParams = new URLSearchParams();
  if (params?.status) searchParams.set("status", params.status);
  if (params?.limit) searchParams.set("limit", String(params.limit));
  if (params?.offset) searchParams.set("offset", String(params.offset));

  const query = searchParams.toString();
  const url = `/v1/durable/schedules/${scheduleId}/executions${query ? `?${query}` : ""}`;
  const response = await api.get<ScheduleExecutionsResponse>(url);
  return response.data;
}

export async function getScheduleExecution(executionId: string): Promise<ScheduleExecution> {
  const response = await api.get<ScheduleExecution>(`/v1/durable/executions/${executionId}`);
  return response.data;
}

export async function getScheduleStats(scheduleId: string): Promise<ScheduleStats> {
  const response = await api.get<ScheduleStats>(`/v1/durable/schedules/${scheduleId}/stats`);
  return response.data;
}
