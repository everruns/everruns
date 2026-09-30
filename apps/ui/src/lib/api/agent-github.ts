import { api } from "./client";

export interface AgentGitHubStatus {
  identity_id: string | null;
  connected: boolean;
  app_created: boolean;
  app_slug: string | null;
  app_url: string | null;
  account: string | null;
  repository_selection: string | null;
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
  await api.delete(`/v1/agent-identities/${identityId}/connections/github/app`);
}

export async function listGitHubRepositories(identityId: string): Promise<GitHubRepository[]> {
  return (
    await api.get<GitHubRepository[]>(
      `/v1/agent-identities/${identityId}/connections/github/repositories`,
    )
  ).data;
}
