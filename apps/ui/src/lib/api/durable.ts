// Durable Execution API functions

import { api } from "./client";
import type {
  WorkersResponse,
  WorkflowsResponse,
  DurableWorkflow,
  WorkflowEvent,
  TasksResponse,
  TaskQueueStats,
  DlqResponse,
  CircuitBreaker,
  DurableSystemHealth,
} from "./types";

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

// ============================================
// Workflows
// ============================================

export interface ListWorkflowsParams {
  status?: string;
  workflow_type?: string;
  limit?: number;
  offset?: number;
}

export async function listWorkflows(
  params?: ListWorkflowsParams
): Promise<WorkflowsResponse> {
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
  const response = await api.get<DurableWorkflow>(
    `/v1/durable/workflows/${workflowId}`
  );
  return response.data;
}

export async function getWorkflowEvents(
  workflowId: string
): Promise<WorkflowEvent[]> {
  const response = await api.get<WorkflowEvent[]>(
    `/v1/durable/workflows/${workflowId}/events`
  );
  return response.data;
}

export async function cancelWorkflow(workflowId: string): Promise<void> {
  await api.post(`/v1/durable/workflows/${workflowId}/cancel`);
}

export async function signalWorkflow(
  workflowId: string,
  signalType: string,
  payload?: Record<string, unknown>
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
  const response = await api.post<{ task_id: string }>(
    `/v1/durable/dlq/${dlqId}/requeue`
  );
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

export async function listCircuitBreakers(): Promise<CircuitBreaker[]> {
  const response = await api.get<CircuitBreaker[]>("/v1/durable/circuit-breakers");
  return response.data;
}

export async function resetCircuitBreaker(key: string): Promise<void> {
  await api.post(`/v1/durable/circuit-breakers/${encodeURIComponent(key)}/reset`);
}
