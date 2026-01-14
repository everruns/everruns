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

// MCP Server hooks

export function useMcpServers() {
  return useQuery({
    queryKey: queryKeys.mcpServers.list(),
    queryFn: getMcpServers,
    staleTime: 30000,
  });
}

export function useMcpServer(serverId: string) {
  return useQuery({
    queryKey: queryKeys.mcpServers.detail(serverId),
    queryFn: () => getMcpServer(serverId),
    enabled: !!serverId,
  });
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
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.detail(serverId) });
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
