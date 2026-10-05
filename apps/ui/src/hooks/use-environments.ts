"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  archiveEnvironment,
  createEnvironment,
  getEnvironment,
  listEnvironments,
  listEnvironmentTargets,
  reviseEnvironment,
  type CreateEnvironmentDefinitionRequest,
} from "@/lib/api/environments";

export function useEnvironmentTargets() {
  return useQuery({
    queryKey: ["environment-targets"],
    queryFn: listEnvironmentTargets,
    staleTime: 60_000,
  });
}

export function useEnvironments() {
  return useQuery({ queryKey: ["environments"], queryFn: listEnvironments });
}

export function useEnvironment(id: string) {
  return useQuery({
    queryKey: ["environment", id],
    queryFn: () => getEnvironment(id),
    enabled: Boolean(id),
  });
}

export function useCreateEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateEnvironmentDefinitionRequest) => createEnvironment(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["environments"] }),
  });
}

export function useReviseEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...request }: Parameters<typeof reviseEnvironment>[1] & { id: string }) =>
      reviseEnvironment(id, request),
    onSuccess: (environment) => {
      queryClient.setQueryData(["environment", environment.id], environment);
      queryClient.invalidateQueries({ queryKey: ["environments"] });
    },
  });
}

export function useArchiveEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: archiveEnvironment,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["environments"] }),
  });
}
