// Agent API functions (M2)
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, getApiBaseUrl, throwApiError } from "./client";
import type { AgentAvatar } from "./agent-types";
import type { AvatarPreset, AvatarPresetSelection } from "./schema-types";
import { createCrudApi } from "./crud";
import { withOrgHeader } from "./active-org";
import type {
  Agent,
  AgentAnalysisResponse,
  AgentPreviewResponse,
  HealthCheckRun,
  LatestHealthCheckRun,
  CreateAgentRequest,
  PreviewAgentRequest,
  ResourceStats,
  PaginatedResponse,
  UpdateAgentRequest,
  AgentCredentialBinding,
  AgentMcpAttachment,
} from "./types";

export const agentsCrudApi = createCrudApi<Agent, CreateAgentRequest, UpdateAgentRequest>(
  "/v1/agents",
);

export const createAgent = agentsCrudApi.create;
export const listAgents = agentsCrudApi.list;
export const getAgent = agentsCrudApi.get;
export const updateAgent = agentsCrudApi.update;
export const deleteAgent = agentsCrudApi.delete;
export const destroyAgent = agentsCrudApi.destroy;

/** Lazy, server-filtered agent search used by browser-native UI integrations. */
export async function searchAgents(
  query: string,
  limit: number,
): Promise<PaginatedResponse<Agent>> {
  const params = new URLSearchParams({ search: query, limit: String(limit) });
  const response = await api.get<PaginatedResponse<Agent>>(`/v1/agents?${params.toString()}`);
  return response.data;
}

export async function exportAgent(agentId: string): Promise<string> {
  // Raw fetch needed: returns text/markdown, not JSON
  const response = await fetch(`/api/v1/agents/${agentId}/export`, {
    credentials: "include",
    headers: withOrgHeader(),
  });
  if (!response.ok) {
    await throwApiError(response);
  }
  return response.text();
}

function packageQuery(file?: File, target?: string): string {
  const params = new URLSearchParams();
  if (file) {
    const extension = file.name.split(".").pop();
    if (extension && ["md", "toml", "yaml", "yml", "json", "zip"].includes(extension))
      params.set("format", extension);
  }
  if (target) params.set("target", target);
  return params.size ? `?${params.toString()}` : "";
}

export type AgentImport = string | { file: File; target?: string };

function packageRequest(input: AgentImport): { body: BodyInit; headers: HeadersInit } {
  const body = typeof input === "string" ? input : input.file;
  return {
    body,
    headers: withOrgHeader({
      "Content-Type":
        body instanceof File && body.name.endsWith(".zip") ? "application/zip" : "text/plain",
    }),
  };
}

export async function importAgent(input: AgentImport): Promise<Agent> {
  const target = typeof input === "string" ? undefined : input.target;
  const response = await fetch(
    `/api/v1/agents/import${packageQuery(typeof input === "string" ? undefined : input.file, target)}`,
    {
      method: "POST",
      credentials: "include",
      ...packageRequest(input),
    },
  );
  if (!response.ok) await throwApiError(response);
  return response.json();
}

export interface AgentPackagePreview {
  name: string;
  display_name?: string;
  description?: string;
  instructions: string;
  model?: { provider: string; model: string };
  harness?: string;
  tags?: string[];
  capabilities: Record<string, unknown>;
  files: Record<string, { bytes: number; is_readonly: boolean; sha256: string }>;
  mcpServers?: Record<string, { use?: string; type?: string; url?: string; command?: string }>;
  channels?: Record<string, { type: string; enabled: boolean; config: unknown }>;
  max_iterations?: number;
  parallel_tool_calls?: boolean;
  tools?: { name: string; description: string; parameters: unknown }[];
  network_access?: unknown;
  environments?: unknown;
  intro_markdown?: string;
  short_description?: string;
  starters?: { text: string; icon?: string }[];
}

export async function inspectAgentPackage(
  file: File,
  operation: "validate" | "diff",
  target?: string,
): Promise<{
  valid?: boolean;
  preview?: AgentPackagePreview;
  diagnostics?: { path: string; message: string }[];
  changes?: { path: string; before: unknown; after: unknown }[];
}> {
  const response = await fetch(`/api/v1/agents/${operation}${packageQuery(file, target)}`, {
    method: "POST",
    credentials: "include",
    ...packageRequest({ file }),
  });
  if (!response.ok) await throwApiError(response);
  return response.json();
}

export async function exportAgentPackage(
  agentName: string,
  format: "zip" | "toml" | "yaml" | "json" = "zip",
): Promise<Blob> {
  const response = await fetch(
    `/api/v1/agents/${encodeURIComponent(agentName)}/export?format=${format}`,
    { credentials: "include", headers: withOrgHeader() },
  );
  if (!response.ok) await throwApiError(response);
  return response.blob();
}

export async function copyAgent(agentId: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agents/${agentId}/copy`, {});
  return response.data;
}

export async function getAgentStats(agentId: string): Promise<ResourceStats> {
  const response = await api.get<ResourceStats>(`/v1/agents/${agentId}/stats`);
  return response.data;
}

export async function getAgentMcpAttachments(agentId: string): Promise<AgentMcpAttachment[]> {
  const response = await api.get<AgentMcpAttachment[]>(
    `/v1/agents/${encodeURIComponent(agentId)}/mcp-attachments`,
  );
  return response.data;
}

export async function revokeAgentMcpConnection(agentId: string, name: string): Promise<void> {
  await api.delete(
    `/v1/agents/${encodeURIComponent(agentId)}/mcp-attachments/${encodeURIComponent(name)}/connection`,
  );
}

export async function checkAgentName(
  name: string,
  excludeId?: string,
): Promise<{ available: boolean }> {
  const params = new URLSearchParams({ name });
  if (excludeId) params.set("exclude_id", excludeId);
  const response = await api.get<{ available: boolean }>(
    `/v1/agents/check-name?${params.toString()}`,
  );
  return response.data;
}

export async function previewAgent(request: PreviewAgentRequest): Promise<AgentPreviewResponse> {
  const response = await api.post<AgentPreviewResponse>("/v1/agents/preview", request);
  return response.data;
}

export async function analyzeAgent(request: PreviewAgentRequest): Promise<AgentAnalysisResponse> {
  const response = await api.post<AgentAnalysisResponse>("/v1/agents/analyze", request);
  return response.data;
}

export async function triggerHealthCheck(agentId: string): Promise<HealthCheckRun> {
  const response = await api.post<HealthCheckRun>(`/v1/agents/${agentId}/health-checks`, {});
  return response.data;
}

export async function getHealthCheckRun(agentId: string, runId: string): Promise<HealthCheckRun> {
  const response = await api.get<HealthCheckRun>(`/v1/agents/${agentId}/health-checks/${runId}`);
  return response.data;
}

export async function getLatestHealthCheckRun(agentId: string): Promise<LatestHealthCheckRun> {
  const response = await api.get<LatestHealthCheckRun>(
    `/v1/agents/${agentId}/health-checks/latest`,
  );
  return response.data;
}

export async function listHealthCheckRuns(agentId: string): Promise<HealthCheckRun[]> {
  const response = await api.get<HealthCheckRun[]>(`/v1/agents/${agentId}/health-checks`);
  return response.data;
}

export interface CreateAgentCredentialBindingRequest {
  mcp_server_name: string;
  tool_name: string;
  parameter_name: string;
  label: string;
  description?: string;
}

export async function listAgentCredentials(agentId: string): Promise<AgentCredentialBinding[]> {
  const response = await api.get<{ data: AgentCredentialBinding[] }>(
    `/v1/agents/${agentId}/credentials`,
  );
  return response.data.data;
}

export async function createAgentCredentialBinding(
  agentId: string,
  request: CreateAgentCredentialBindingRequest,
): Promise<AgentCredentialBinding> {
  const response = await api.post<AgentCredentialBinding>(
    `/v1/agents/${agentId}/credentials`,
    request,
  );
  return response.data;
}

export async function setAgentCredentialValue(
  agentId: string,
  bindingId: string,
  value: string,
): Promise<AgentCredentialBinding> {
  const response = await api.put<AgentCredentialBinding>(
    `/v1/agents/${agentId}/credentials/${bindingId}`,
    { value },
  );
  return response.data;
}

export async function deleteAgentCredentialBinding(
  agentId: string,
  bindingId: string,
): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/credentials/${bindingId}`);
}

/// The agent-level incident switch (EVE-1007). One call takes every channel of
/// an agent off the internet without touching the per-channel publish state it
/// should be restored to.
export async function suspendAgentExposures(agentId: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agents/${agentId}/exposures/suspend`);
  return response.data;
}

export async function resumeAgentExposures(agentId: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agents/${agentId}/exposures/resume`);
  return response.data;
}

/** Upload (or replace) an agent's avatar. The server crops it square and renders presets. */
export async function uploadAgentAvatar(agentId: string, file: File): Promise<AgentAvatar> {
  const formData = new FormData();
  formData.append("file", file);
  // Raw fetch for FormData: the browser sets the multipart boundary.
  const response = await fetch(`${getApiBaseUrl()}/v1/agents/${agentId}/avatar`, {
    method: "PUT",
    body: formData,
    credentials: "include",
    headers: withOrgHeader(),
  });
  if (!response.ok) await throwApiError(response);
  return response.json();
}

export async function deleteAgentAvatar(agentId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/avatar`);
}

/** Browser URL of one avatar preset. Sizes come from `avatar.sizes`. */
export function agentAvatarUrl(
  avatar: AgentAvatar,
  size: number,
  shape: "square" | "circle" = "square",
): string {
  const preset = avatar.sizes.includes(size)
    ? size
    : (avatar.sizes.find((s) => s >= size) ?? avatar.sizes[avatar.sizes.length - 1]);
  return `${getApiBaseUrl()}/v1/avatars/${avatar.id}/${shape}-${preset}.png`;
}

/** Curated presentation metadata; choosing a role never changes agent behavior. */
export type { AvatarPreset } from "./schema-types";

export async function listAvatarPresets(): Promise<AvatarPreset[]> {
  return (await api.get<AvatarPreset[]>("/v1/avatar-presets")).data;
}
export async function getAvatarPresetSelection(agentId: string): Promise<AvatarPresetSelection> {
  return (await api.get<AvatarPresetSelection>(`/v1/agents/${agentId}/avatar/preset`)).data;
}
export async function selectAgentAvatarPreset(
  agentId: string,
  presetId: string,
): Promise<AgentAvatar> {
  return (
    await api.put<AgentAvatar>(`/v1/agents/${agentId}/avatar/preset`, { preset_id: presetId })
  ).data;
}
export function avatarPresetUrl(
  preset: AvatarPreset,
  shape: "square" | "circle" = "square",
): string {
  return `${getApiBaseUrl()}/v1/avatar-presets/${encodeURIComponent(preset.id)}/${shape}-256.png`;
}
