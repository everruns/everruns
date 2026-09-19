"use client";

import { getMcpServerCatalog, getMcpServerUsage, mcpServersCrudApi } from "@/lib/api/mcp-servers";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { CreateMcpServerRequest, UpdateMcpServerRequest } from "@/lib/api/types";
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
