import { api } from "./client";
import type {
  MonitoringSnapshot,
  MonitoringActorPage,
  MonitoringActorDetail,
  MonitoringTimeline,
  MonitoringTimelineWindow,
} from "./types";

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

export type TimelineRange = "15m" | "1h" | "6h" | "24h" | "7d" | "30d";
export interface TimelineBounds {
  range: TimelineRange;
  /** Custom bounds from a zoomed selection; override `range` when set. */
  from?: string;
  to?: string;
}
export async function getMonitoringTimeline(
  scope: MonitoringScope,
  bounds: TimelineBounds,
): Promise<MonitoringTimeline> {
  const params = new URLSearchParams();
  if (bounds.from && bounds.to) {
    params.set("from", bounds.from);
    params.set("to", bounds.to);
  } else params.set("range", bounds.range);
  return (await api.get<MonitoringTimeline>(`${monitoringBase(scope)}/timeline?${params}`)).data;
}
export async function getMonitoringTimelineWindow(
  scope: MonitoringScope,
  from: string,
  to: string,
): Promise<MonitoringTimelineWindow> {
  const params = new URLSearchParams({ from, to });
  return (
    await api.get<MonitoringTimelineWindow>(`${monitoringBase(scope)}/timeline/window?${params}`)
  ).data;
}
