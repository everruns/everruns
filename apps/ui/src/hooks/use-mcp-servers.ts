"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getMcpServers,
  getMcpServer,
  createMcpServer,
  updateMcpServer,
  deleteMcpServer,
} from "@/lib/api/mcp-servers";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateMcpServerRequest,
  UpdateMcpServerRequest,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// MCP Server hooks

export function useMcpServers() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.mcpServers.list(), org],
    queryFn: () => getMcpServers(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useMcpServer(serverId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.mcpServers.detail(serverId), org],
    queryFn: () => getMcpServer(serverId),
    enabled: !!org && !!serverId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateMcpServer() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateMcpServerRequest) => createMcpServer(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
    },
  });
}

export function useUpdateMcpServer(serverId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateMcpServerRequest) =>
      updateMcpServer(serverId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.mcpServers.detail(serverId),
      });
    },
  });
}

export function useDeleteMcpServer() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (serverId: string) => deleteMcpServer(serverId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
    },
  });
}
