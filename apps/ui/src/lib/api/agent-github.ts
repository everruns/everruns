import { api } from "./client";
import type { GitHubOwnerSuggestion } from "./schema-types";

export interface AgentGitHubStatus {
  identity_id: string | null;
  connected: boolean;
  app_created: boolean;
  app_slug: string | null;
  app_url: string | null;
  account: string | null;
  repository_selection: string | null;
  /** Permissions new Apps request that this installation lacks, as `name:level`. */
  missing_permissions: string[];
  /** GitHub accounts this organization already reached, candidates for `owner_org`. */
  suggested_owners: GitHubOwnerSuggestion[];
}

export type AgentGitHubConnectResponse =
  | { kind: "create_app"; action: string; manifest: string }
  | { kind: "install"; url: string; app_slug: string };

export interface GitHubRepository {
  full_name: string;
  private: boolean;
  html_url: string;
}

export async function getAgentGitHub(agentId: string): Promise<AgentGitHubStatus> {
  return (await api.get<AgentGitHubStatus>(`/v1/agents/${agentId}/github`)).data;
}

export async function connectAgentGitHub(
  agentId: string,
  body: { return_to?: string; owner_org?: string },
): Promise<AgentGitHubConnectResponse> {
  return (await api.post<AgentGitHubConnectResponse>(`/v1/agents/${agentId}/github/connect`, body))
    .data;
}

export async function disconnectAgentGitHub(identityId: string): Promise<void> {
  await api.delete(`/v1/virtual-users/${identityId}/connections/github/app`);
}

export async function listGitHubRepositories(identityId: string): Promise<GitHubRepository[]> {
  return (
    await api.get<GitHubRepository[]>(
      `/v1/virtual-users/${identityId}/connections/github/repositories`,
    )
  ).data;
}
