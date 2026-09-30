"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  connectAgentGitHub,
  disconnectAgentGitHub,
  getAgentGitHub,
  listGitHubRepositories,
  type AgentGitHubConnectResponse,
} from "@/lib/api/agent-github";
import { navigateTo } from "@/lib/browser-navigation";
import { queryKeys } from "@/lib/query-keys";

export function useAgentGitHub(agentId: string) {
  return useQuery({
    queryKey: queryKeys.agentGithub.status(agentId),
    queryFn: () => getAgentGitHub(agentId),
  });
}

export function useGitHubRepositories(identityId: string | null | undefined, enabled = true) {
  return useQuery({
    queryKey: queryKeys.agentGithub.repositories(identityId ?? ""),
    queryFn: () => listGitHubRepositories(identityId as string),
    enabled: enabled && !!identityId,
  });
}

/// Sends the browser to GitHub: a top-level form POST of the App manifest when
/// the App must be created, otherwise a plain redirect to the install page.
export function redirectToGitHub(response: AgentGitHubConnectResponse) {
  if (response.kind === "install") {
    navigateTo(response.url);
    return;
  }
  const form = document.createElement("form");
  form.method = "post";
  form.action = response.action;
  form.style.display = "none";
  const input = document.createElement("input");
  input.type = "hidden";
  input.name = "manifest";
  input.value = response.manifest;
  form.appendChild(input);
  document.body.appendChild(form);
  form.submit();
}

export function useConnectAgentGitHub(agentId: string) {
  return useMutation({
    mutationFn: () =>
      connectAgentGitHub(agentId, { return_to: `/agents/${agentId}?tab=integrations` }),
    onSuccess: redirectToGitHub,
  });
}

export function useDisconnectAgentGitHub(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (identityId: string) => disconnectAgentGitHub(identityId),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.agentGithub.status(agentId) }),
  });
}
