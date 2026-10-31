import { api, getBackendUrl } from "./client";
import type { AgentPackagePreview } from "./agents";

/** Development GitHub account a source reads through, never the Agent's runtime identity. */
export type SourceGitHubAccount = "personal" | "organization";
export interface AgentSource {
  mode: "pull" | "two_way";
  /** `agent` marks a binding made before sync stopped using the Agent's identity. */
  github_account: SourceGitHubAccount | "agent";
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
export interface SourceRepository {
  full_name: string;
  account: string;
  private: boolean;
}
export interface AgentSourceAccounts {
  available: boolean;
  install_url: string | null;
  personal: {
    connected: boolean;
    login: string | null;
    repositories: SourceRepository[];
    error: string | null;
  };
  organization: {
    can_connect: boolean;
    accounts: string[];
    repositories: SourceRepository[];
    error: string | null;
  };
}
const route = (id: string) => `/v1/agents/${encodeURIComponent(id)}/source`;
export async function getAgentSource(id: string): Promise<AgentSource | null> {
  return (await api.get<AgentSource | null>(route(id))).data;
}
export async function listAgentSourceAccounts(id: string): Promise<AgentSourceAccounts> {
  return (await api.get<AgentSourceAccounts>(`${route(id)}/accounts`)).data;
}
/** Browser targets that connect a development account in a popup. */
export function personalGitHubSignInUrl(returnTo: string): string {
  return `${getBackendUrl()}/v1/user/connections/github/authorize?popup=true&return_to=${encodeURIComponent(returnTo)}`;
}
export function organizationGitHubLinkUrl(org: string, returnTo: string): string {
  return `${getBackendUrl()}/v1/orgs/${encodeURIComponent(org)}/github/everruns-app/authorize?popup=true&return_to=${encodeURIComponent(returnTo)}`;
}
export async function getAgentSourceOptions(
  id: string,
  location: {
    repository: string;
    branch?: string;
    path?: string;
    github_account?: SourceGitHubAccount;
  },
): Promise<SourceOptions> {
  return (await api.post<SourceOptions>(`${route(id)}/options`, location)).data;
}
export async function previewAgentSource(
  id: string,
  location: SourceLocation & { github_account?: SourceGitHubAccount },
): Promise<SourcePreview> {
  return (await api.post<SourcePreview>(`${route(id)}/preview`, location)).data;
}
export async function connectAgentSource(
  id: string,
  body: SourceLocation & {
    mode?: "pull" | "two_way";
    github_account?: SourceGitHubAccount;
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
