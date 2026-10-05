"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  archiveSandboxTemplate,
  createSandboxTemplate,
  getSandboxTemplate,
  listSandboxTargets,
  listSandboxTemplates,
  reviseSandboxTemplate,
  type CreateSandboxTemplateRequest,
} from "@/lib/api/sandbox-templates";

const templateListKey = ["sandbox-templates"] as const;

export function useSandboxTargets() {
  return useQuery({
    queryKey: ["sandbox-targets"],
    queryFn: listSandboxTargets,
    staleTime: 60_000,
  });
}

export function useSandboxTemplates() {
  return useQuery({ queryKey: templateListKey, queryFn: listSandboxTemplates });
}

export function useSandboxTemplate(id: string) {
  return useQuery({
    queryKey: ["sandbox-template", id],
    queryFn: () => getSandboxTemplate(id),
    enabled: Boolean(id),
  });
}

export function useCreateSandboxTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateSandboxTemplateRequest) => createSandboxTemplate(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: templateListKey }),
  });
}

export function useReviseSandboxTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      ...request
    }: Parameters<typeof reviseSandboxTemplate>[1] & { id: string }) =>
      reviseSandboxTemplate(id, request),
    onSuccess: (template) => {
      queryClient.setQueryData(["sandbox-template", template.id], template);
      queryClient.invalidateQueries({ queryKey: templateListKey });
    },
  });
}

export function useArchiveSandboxTemplate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: archiveSandboxTemplate,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: templateListKey }),
  });
}
