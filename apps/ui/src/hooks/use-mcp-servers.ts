"use client";

import {
  checkMcpServerConnection,
  getMcpServerCatalog,
  getMcpServerTools,
  getMcpServerUsage,
  mcpServersCrudApi,
  setMcpToolLabel,
  suggestMcpToolLabels,
} from "@/lib/api/mcp-servers";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  CreateMcpServerRequest,
  McpServerTool,
  McpToolLabel,
  UpdateMcpServerRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import { createCrudHooks } from "./create-crud-hooks";

// MCP Server hooks

const mcpServerCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof mcpServersCrudApi.get>>,
  CreateMcpServerRequest,
  UpdateMcpServerRequest
>({
  api: mcpServersCrudApi,
  queryKeys: queryKeys.mcpServers,
  staleTime: 30000,
});

export const useMcpServers = mcpServerCrudHooks.useList;
export const useMcpServer = mcpServerCrudHooks.useDetail;
export const useCreateMcpServer = mcpServerCrudHooks.useCreate;
export const useDeleteMcpServer = mcpServerCrudHooks.useDelete;
export const useDestroyMcpServer = mcpServerCrudHooks.useDestroy;

export function useMcpServerCatalog(enabled = true) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useInfiniteQuery({
    queryKey: queryKeys.mcpServers.catalog(org),
    queryFn: ({ pageParam }) => getMcpServerCatalog(pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: enabled && !!org,
    staleTime: 30000,
  });
  return {
    ...query,
    data: query.data?.pages.flatMap((page) => page.data),
    isLoading: orgLoading || query.isLoading,
  };
}

export function useMcpServerUsage(serverId?: string) {
  return useQuery({
    queryKey: queryKeys.mcpServers.usage(serverId ?? ""),
    queryFn: () => getMcpServerUsage(serverId!),
    enabled: !!serverId,
    staleTime: 0,
  });
}

export function useUpdateMcpServer(serverId: string) {
  const mutation = mcpServerCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (request: UpdateMcpServerRequest, options?: Parameters<typeof mutation.mutate>[1]) =>
      mutation.mutate({ id: serverId, request }, options),
    mutateAsync: (
      request: UpdateMcpServerRequest,
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: serverId, request }, options),
  };
}

export function useMcpServerTools(serverId?: string) {
  return useQuery({
    queryKey: queryKeys.mcpServers.tools(serverId ?? ""),
    queryFn: () => getMcpServerTools(serverId!),
    enabled: !!serverId,
    staleTime: 0,
  });
}

export function useSetMcpToolLabel(serverId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ toolName, label }: { toolName: string; label: McpToolLabel | null }) =>
      setMcpToolLabel(serverId, toolName, label),
    onSuccess: (saved) => {
      queryClient.setQueryData<McpServerTool[]>(queryKeys.mcpServers.tools(serverId), (tools) =>
        tools?.map((tool) => (tool.name === saved.name ? saved : tool)),
      );
    },
  });
}

export function useSuggestMcpToolLabels(serverId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => suggestMcpToolLabels(serverId),
    onSuccess: (tools) => {
      queryClient.setQueryData(queryKeys.mcpServers.tools(serverId), tools);
    },
  });
}

/** "Check again" for an OAuth catalog preset; refreshes the catalog with the result. */
export function useCheckMcpServerConnection() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (serverId: string) => checkMcpServerConnection(serverId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
    },
  });
}
