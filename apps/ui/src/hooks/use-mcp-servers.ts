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
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.mcpServers.list(), org],
    queryFn: () => getMcpServers(org!),
    enabled: !!org,
    staleTime: 30000,
  });
}

export function useMcpServer(serverId: string) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.mcpServers.detail(serverId), org],
    queryFn: () => getMcpServer(org!, serverId),
    enabled: !!org && !!serverId,
  });
}

export function useCreateMcpServer() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: CreateMcpServerRequest) => createMcpServer(org!, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
    },
  });
}

export function useUpdateMcpServer(serverId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: UpdateMcpServerRequest) =>
      updateMcpServer(org!, serverId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.detail(serverId) });
    },
  });
}

export function useDeleteMcpServer() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (serverId: string) => deleteMcpServer(org!, serverId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.all });
    },
  });
}
