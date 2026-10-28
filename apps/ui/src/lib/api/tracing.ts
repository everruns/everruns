import { api } from "./client";
export type RoutingMode = "inherit" | "add" | "replace" | "off";
export type TracingPolicy = { mode: RoutingMode; destination_ids: string[] };
export interface TracingPolicyView {
  policy: TracingPolicy;
  revision: number;
}
export interface TracingConfig {
  name: string;
  provider: "braintrust" | "otlp";
  endpoint: string;
  project_id: string | null;
  capture_content: boolean;
  capture_reasoning: boolean;
  enabled: boolean;
}
export interface TracingDestination {
  id: string;
  config: TracingConfig;
  revision: number;
  last_accepted_at: string | null;
  last_attempt_at: string | null;
  last_error: string | null;
  dropped_events: number;
  organization_default: boolean;
  agent_selections: number;
}
export interface SaveDestination {
  config: TracingConfig;
  revision?: number;
  secrets?: { headers: Record<string, string> };
}
export const tracingPolicyPath = (agent?: string) =>
  agent ? `/v1/agents/${encodeURIComponent(agent)}/tracing` : "/v1/tracing/defaults";
export const listTracingDestinations = async () =>
  (await api.get<TracingDestination[]>("/v1/tracing/destinations")).data;
export const getTracingPolicy = async (agent?: string) =>
  (await api.get<TracingPolicyView>(tracingPolicyPath(agent))).data;
export const saveTracingPolicy = async (value: TracingPolicyView, agent?: string) =>
  (await api.put<TracingPolicyView>(tracingPolicyPath(agent), value)).data;
export const saveTracingDestination = async (value: SaveDestination, id?: string) =>
  id
    ? (await api.put<TracingDestination>(`/v1/tracing/destinations/${id}`, value)).data
    : (await api.post<TracingDestination>("/v1/tracing/destinations", value)).data;
export const deleteTracingDestination = async (id: string, revision: number) =>
  api.delete(`/v1/tracing/destinations/${id}?revision=${revision}`);
export const testTracingDestination = async (id: string) =>
  (await api.post<{ accepted: boolean; message: string }>(`/v1/tracing/destinations/${id}/test`))
    .data;
export function effectiveDestinations(policy: TracingPolicy, defaults: string[]): string[] {
  return [
    ...new Set([
      ...(policy.mode === "inherit" || policy.mode === "add" ? defaults : []),
      ...(policy.mode === "add" || policy.mode === "replace" ? policy.destination_ids : []),
    ]),
  ];
}
