"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getMcpServers,
  getMcpServer,
  createMcpServer,
  updateMcpServer,
  deleteMcpServer,
  getMcpServerOAuthStatus,
  startMcpServerOAuth,
  revokeMcpServerOAuth,
} from "@/lib/api/mcp-servers";
import { queryKeys } from "@/lib/query-keys";
import type { CreateMcpServerRequest, McpOAuthStatus, UpdateMcpServerRequest } from "@/lib/api/types";
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
    mutationFn: (data: UpdateMcpServerRequest) => updateMcpServer(serverId, data),
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

// MCP Server OAuth hooks

export function useMcpServerOAuthStatus(serverId: string) {
  const { data: oauthStatus, ...rest } = useQuery({
    queryKey: queryKeys.mcpServers.oauthStatus(serverId),
    queryFn: () => getMcpServerOAuthStatus(serverId),
    enabled: !!serverId,
    // Use token expiry for smart refetch: poll more frequently near expiry
    staleTime: 30_000,
    refetchInterval: (query) => {
      const data = query.state.data as McpOAuthStatus | undefined;
      if (!data?.authorized || !data?.expires_at) return false;
      const expiresIn = new Date(data.expires_at).getTime() - Date.now();
      // Poll every 30s when token expires within 5 minutes
      if (expiresIn < 5 * 60 * 1000) return 30_000;
      return false;
    },
  });

  return { data: oauthStatus, ...rest };
}

export function useStartMcpServerOAuth(serverId: string) {
  return useMutation({
    mutationFn: (returnUrl?: string) => startMcpServerOAuth(serverId, returnUrl),
  });
}

export function useRevokeMcpServerOAuth(serverId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => revokeMcpServerOAuth(serverId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.oauthStatus(serverId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers.detail(serverId) });
    },
  });
}
