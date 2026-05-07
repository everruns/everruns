// Agent hooks (M2)
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  agentsCrudApi,
  copyAgent,
  createAgentVersion,
  diffAgentVersions,
  exportAgent,
  forkAgentVersion,
  getAgentStats,
  importAgent,
  listAgentVersions,
  previewAgent,
  rollbackAgentVersion,
  setDefaultAgentVersion,
} from "@/lib/api/agents";
import type {
  CreateAgentRequest,
  CreateAgentVersionRequest,
  ForkAgentVersionRequest,
  PreviewAgentRequest,
  RollbackAgentVersionRequest,
  SetDefaultAgentVersionRequest,
  UpdateAgentRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";
import { useOrg } from "@/providers/org-provider";

const agentCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof agentsCrudApi.get>>,
  CreateAgentRequest,
  UpdateAgentRequest
>({
  api: agentsCrudApi,
  queryKeys: queryKeys.agents,
});

export const useAgents = agentCrudHooks.useList;
export const useAgent = agentCrudHooks.useDetail;
export const useCreateAgent = agentCrudHooks.useCreate;
export const useDeleteAgent = agentCrudHooks.useDelete;
export const useDestroyAgent = agentCrudHooks.useDestroy;

export function useAgentStats(agentId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.agents.stats(org, agentId),
    queryFn: () => getAgentStats(agentId!),
    enabled: !!org && !!agentId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useAgentVersions(agentId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.agents.versions(org, agentId),
    queryFn: () => listAgentVersions(agentId!),
    enabled: !!org && !!agentId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useUpdateAgent() {
  const mutation = agentCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { agentId: string; request: UpdateAgentRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.agentId, request: variables.request }, options),
    mutateAsync: (
      variables: { agentId: string; request: UpdateAgentRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.agentId, request: variables.request }, options),
  };
}

export function useCopyAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: copyAgent,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useExportAgent() {
  return useMutation({
    mutationFn: exportAgent,
  });
}

export function useImportAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: importAgent,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function usePreviewAgent() {
  return useMutation({
    mutationFn: (request: PreviewAgentRequest) => previewAgent(request),
  });
}

function invalidateAgentVersions(
  queryClient: ReturnType<typeof useQueryClient>,
  org: string | undefined,
  agentId: string,
) {
  queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
  queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) });
  queryClient.invalidateQueries({ queryKey: queryKeys.agents.versions(org, agentId) });
  queryClient.invalidateQueries({ queryKey: ["agent", org, agentId, "versions", "diff"] });
}

export function useCreateAgentVersion() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ agentId, request }: { agentId: string; request: CreateAgentVersionRequest }) =>
      createAgentVersion(agentId, request),
    onSuccess: (_version, variables) => {
      invalidateAgentVersions(queryClient, org, variables.agentId);
    },
  });
}

export function useSetDefaultAgentVersion() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request: SetDefaultAgentVersionRequest;
    }) => setDefaultAgentVersion(agentId, request),
    onSuccess: (agent) => {
      queryClient.setQueryData(queryKeys.agents.detail(agent.id), agent);
      invalidateAgentVersions(queryClient, org, agent.id);
    },
  });
}

export function useRollbackAgentVersion() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      versionId,
      request,
    }: {
      agentId: string;
      versionId: string;
      request: RollbackAgentVersionRequest;
    }) => rollbackAgentVersion(agentId, versionId, request),
    onSuccess: (agent) => {
      queryClient.setQueryData(queryKeys.agents.detail(agent.id), agent);
      invalidateAgentVersions(queryClient, org, agent.id);
    },
  });
}

export function useForkAgentVersion() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      versionId,
      request,
    }: {
      agentId: string;
      versionId: string;
      request: ForkAgentVersionRequest;
    }) => forkAgentVersion(agentId, versionId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useAgentVersionDiff(
  agentId: string | undefined,
  fromVersionId: string | undefined,
  toVersionId: string | undefined,
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.agents.versionDiff(org, agentId, fromVersionId, toVersionId),
    queryFn: () => diffAgentVersions(agentId!, fromVersionId!, toVersionId!),
    enabled: !!org && !!agentId && !!fromVersionId && !!toVersionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}
