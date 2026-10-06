// Org-wide Sandbox fleet: every logical Sandbox, its roll-ups and lifecycle.
//
// Lifecycle actions go through the owning Session's Sandbox endpoint; the
// fleet itself is read-only.

import { api } from "./client";
import type {
  ManageSessionSandboxResponse,
  SandboxFleetDetail,
  SandboxFleetItem,
  SandboxFleetPage,
  SandboxFleetStats,
  SandboxStateSpan,
  SandboxTimeline,
  SandboxTimelineLane,
} from "./types";

export type {
  SandboxFleetDetail,
  SandboxFleetItem,
  SandboxFleetPage,
  SandboxFleetStats,
  SandboxStateSpan,
  SandboxTimeline,
  SandboxTimelineLane,
};

export type SandboxFleetState =
  | "running"
  | "paused"
  | "lost"
  | "starting"
  | "failed"
  | "not_started"
  | "deleted";

export type SandboxAttention =
  | "lost"
  | "failed"
  | "init_failed"
  | "idle_running"
  | "cleanup_failed";

export type SandboxAction = "pause" | "resume" | "delete";

/** Filters shared by the list, roll-ups and timeline. */
export interface SandboxFleetFilters {
  /** A single state, `live`, or `all`. */
  state?: string;
  provider?: string;
  needsAttention?: boolean;
  search?: string;
}

function filterParams(filters: SandboxFleetFilters): URLSearchParams {
  const params = new URLSearchParams();
  if (filters.state && filters.state !== "all") params.set("state", filters.state);
  if (filters.provider && filters.provider !== "all") params.set("provider", filters.provider);
  if (filters.needsAttention) params.set("needs_attention", "true");
  const search = filters.search?.trim();
  if (search) params.set("search", search);
  return params;
}

export async function listSandboxes(
  filters: SandboxFleetFilters,
  page: { limit: number; offset: number },
): Promise<SandboxFleetPage> {
  const params = filterParams(filters);
  params.set("limit", String(page.limit));
  params.set("offset", String(page.offset));
  return (await api.get<SandboxFleetPage>(`/v1/sandboxes?${params}`)).data;
}

export async function getSandboxFleetStats(
  filters: SandboxFleetFilters,
): Promise<SandboxFleetStats> {
  const params = filterParams(filters);
  return (await api.get<SandboxFleetStats>(`/v1/sandboxes/stats?${params}`)).data;
}

export async function getSandboxTimeline(
  filters: SandboxFleetFilters,
  window: { from: string; to: string },
): Promise<SandboxTimeline> {
  const params = filterParams(filters);
  params.set("from", window.from);
  params.set("to", window.to);
  return (await api.get<SandboxTimeline>(`/v1/sandboxes/timeline?${params}`)).data;
}

export async function getSandbox(id: string): Promise<SandboxFleetDetail> {
  return (await api.get<SandboxFleetDetail>(`/v1/sandboxes/${id}`)).data;
}

export async function manageSessionSandbox(
  sessionId: string,
  action: SandboxAction,
): Promise<ManageSessionSandboxResponse> {
  return (
    await api.post<ManageSessionSandboxResponse>(`/v1/sessions/${sessionId}/sandbox`, { action })
  ).data;
}
