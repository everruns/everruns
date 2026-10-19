import { api } from "./client";
import type { MonitoringSnapshot, MonitoringActorPage, MonitoringActorDetail } from "./types";

export type MonitoringScope = "organization" | "fleet";
export function monitoringBase(scope: MonitoringScope) {
  return scope === "fleet" ? "/v1/durable/monitoring" : "/v1/monitoring";
}
export async function getMonitoring(scope: MonitoringScope): Promise<MonitoringSnapshot> {
  return (await api.get<MonitoringSnapshot>(monitoringBase(scope))).data;
}
export async function getMonitoringActors(
  scope: MonitoringScope,
  offset: number,
  worker?: string,
  phase?: string,
): Promise<MonitoringActorPage> {
  const params = new URLSearchParams({ offset: String(offset) });
  if (worker) params.set("worker_id", worker);
  if (phase) params.set("phase", phase);
  return (await api.get<MonitoringActorPage>(`${monitoringBase(scope)}/actors?${params}`)).data;
}
export async function getMonitoringActor(
  scope: MonitoringScope,
  session: string,
): Promise<MonitoringActorDetail> {
  return (
    await api.get<MonitoringActorDetail>(
      `${monitoringBase(scope)}/actors/${encodeURIComponent(session)}`,
    )
  ).data;
}
