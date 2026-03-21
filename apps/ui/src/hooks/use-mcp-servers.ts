"use client";

import { mcpServersCrudApi } from "@/lib/api/mcp-servers";
import type { CreateMcpServerRequest, UpdateMcpServerRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
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
