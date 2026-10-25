import { api } from "./client";
import type { AgentPackagePreview } from "./agents";

export interface AgentSource {
  mode: "pull" | "two_way";
  local_dirty: boolean;
  generation: string;
  repository: string;
  branch: string;
  path: string;
  managed_channels: string[];
  applied_commit: string | null;
  status: "pending" | "syncing" | "synced" | "error" | "conflict";
  error: string | null;
  last_checked_at: string | null;
  last_synced_at: string | null;
}
export interface SourceLocation {
  repository: string;
  branch: string;
  path: string;
}
export interface SourcePreview {
  valid: boolean;
  preview?: AgentPackagePreview;
  diagnostics?: { path: string; message: string }[];
  changes?: { path: string; before: unknown; after: unknown }[];
  commit: string;
  agent_digest: string;
}
export interface SourceOptions {
  default_branch: string;
  branch: string;
  branches: string[];
  paths: string[];
}
const route = (id: string) => `/v1/agents/${encodeURIComponent(id)}/source`;
export async function getAgentSource(id: string): Promise<AgentSource | null> {
  return (await api.get<AgentSource | null>(route(id))).data;
}
export async function getAgentSourceOptions(
  id: string,
  location: { repository: string; branch?: string; path?: string },
): Promise<SourceOptions> {
  return (await api.post<SourceOptions>(`${route(id)}/options`, location)).data;
}
export async function previewAgentSource(
  id: string,
  location: SourceLocation,
): Promise<SourcePreview> {
  return (await api.post<SourcePreview>(`${route(id)}/preview`, location)).data;
}
export async function connectAgentSource(
  id: string,
  body: SourceLocation & {
    mode?: "pull" | "two_way";
    commit: string;
    agent_digest: string;
    expected_generation?: string;
  },
): Promise<AgentSource> {
  return (await api.put<AgentSource>(route(id), body)).data;
}
export async function disconnectAgentSource(id: string): Promise<void> {
  await api.delete(route(id));
}
export async function syncAgentSource(id: string): Promise<AgentSource> {
  return (await api.post<AgentSource>(`${route(id)}/sync`, {})).data;
}
