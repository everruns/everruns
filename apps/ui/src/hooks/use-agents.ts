"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getAgents,
  getAgent,
  createAgent,
  updateAgent,
  getAgentVersions,
  createAgentVersion,
} from "@/lib/api/agents";
import type {
  CreateAgentRequest,
  UpdateAgentRequest,
  CreateAgentVersionRequest,
} from "@/lib/api/types";

export function useAgents() {
  return useQuery({
    queryKey: ["agents"],
    queryFn: getAgents,
    staleTime: 30000, // 30 seconds
  });
}

export function useAgent(agentId: string) {
  return useQuery({
    queryKey: ["agents", agentId],
    queryFn: () => getAgent(agentId),
    enabled: !!agentId,
  });
}

export function useCreateAgent() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateAgentRequest) => createAgent(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
  });
}

export function useUpdateAgent(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: UpdateAgentRequest) => updateAgent(agentId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      queryClient.invalidateQueries({ queryKey: ["agents", agentId] });
    },
  });
}

export function useAgentVersions(agentId: string) {
  return useQuery({
    queryKey: ["agents", agentId, "versions"],
    queryFn: () => getAgentVersions(agentId),
    enabled: !!agentId,
    staleTime: 60000, // 1 minute
  });
}

export function useCreateAgentVersion(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateAgentVersionRequest) =>
      createAgentVersion(agentId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["agents", agentId, "versions"],
      });
    },
  });
}
