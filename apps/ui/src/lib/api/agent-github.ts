import { api } from "./client";
import type { GitHubOwnerSuggestion } from "./schema-types";

export interface AgentGitHubStatus {
  identity_id: string | null;
  connected: boolean;
  app_created: boolean;
  app_slug: string | null;
  app_url: string | null;
  /** `agent_app` (the agent's own App) or `everruns_app` (the shared Everruns App). */
  kind: "agent_app" | "everruns_app" | null;
  /** Login GitHub shows for the agent, e.g. `pr-summarizer[bot]`. */
  bot_login: string | null;
  /** The App's settings page on GitHub, where its logo is uploaded. */
  app_settings_url: string | null;
  account: string | null;
  repository_selection: string | null;
  /** Permissions new Apps request that this installation lacks, as `name:level`. */
  missing_permissions: string[];
  /** GitHub accounts this organization already reached, candidates for `owner_org`. */
  suggested_owners: GitHubOwnerSuggestion[];
  /** Shared Everruns App installations an org admin linked, which this agent may choose. */
  everruns_app_installations: EverrunsAppInstallation[];
}

export interface EverrunsAppInstallation {
  connection_id: string;
  account: string;
  account_type: string | null;
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

/** Make the agent act as the shared Everruns App through an org-linked installation. */
export async function chooseAgentEverrunsApp(
  agentId: string,
  connectionId: string,
): Promise<AgentGitHubStatus> {
  return (
    await api.post<AgentGitHubStatus>(`/v1/agents/${agentId}/github/everruns-app`, {
      connection_id: connectionId,
    })
  ).data;
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
