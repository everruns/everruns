"use client";

// Queries and mutations behind the History and Manager notes sheets.
// Keyed by entity ref, so one set of hooks serves every entity kind.

import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  diffEntityRevisions,
  getEntityRevision,
  getManagerContext,
  listEntityHistory,
  restoreEntityRevision,
  setManagerContext,
} from "@/lib/api/change-history";
import { useOrg } from "@/providers/org-provider";

export const changeHistoryKeys = {
  all: (entityRef: string) => ["entity-history", entityRef] as const,
  list: (entityRef: string, org?: string) => ["entity-history", entityRef, "list", org] as const,
  revision: (entityRef: string, revision: number, org?: string) =>
    ["entity-history", entityRef, "revision", revision, org] as const,
  diff: (entityRef: string, from: number, org?: string) =>
    ["entity-history", entityRef, "diff", from, org] as const,
  context: (entityRef: string, org?: string) => ["manager-context", entityRef, org] as const,
};

export function useEntityHistory(entityRef: string, enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useInfiniteQuery({
    queryKey: changeHistoryKeys.list(entityRef, org),
    queryFn: ({ pageParam }) => listEntityHistory(entityRef, pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.before,
    enabled: enabled && !!org,
  });
}

export function useEntityRevision(entityRef: string, revision: number | null) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: changeHistoryKeys.revision(entityRef, revision ?? -1, org),
    queryFn: () => getEntityRevision(entityRef, revision!),
    enabled: !!org && revision !== null,
  });
}

/** Diff of one revision against the entity's current state. */
export function useEntityRevisionDiff(entityRef: string, revision: number | null) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: changeHistoryKeys.diff(entityRef, revision ?? -1, org),
    queryFn: () => diffEntityRevisions(entityRef, revision!),
    enabled: !!org && revision !== null,
  });
}

export function useRestoreEntityRevision(entityRef: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ revision, reason }: { revision: number; reason: string }) =>
      restoreEntityRevision(entityRef, revision, reason),
    // A restore rewrites the entity itself, and its detail lives under a key
    // only its page knows; refetch everything active rather than guess.
    onSuccess: () => queryClient.invalidateQueries(),
  });
}

export function useManagerContext(entityRef: string, enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: changeHistoryKeys.context(entityRef, org),
    queryFn: () => getManagerContext(entityRef),
    enabled: enabled && !!org,
  });
}

export function useSetManagerContext(entityRef: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      content,
      expectedRevision,
      reason,
    }: {
      content: string;
      expectedRevision: number;
      reason?: string;
    }) => setManagerContext(entityRef, content, expectedRevision, reason),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["manager-context", entityRef] });
      // A notes write is itself a recorded change.
      queryClient.invalidateQueries({ queryKey: changeHistoryKeys.all(entityRef) });
    },
  });
}
