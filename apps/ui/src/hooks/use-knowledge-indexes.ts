"use client";

import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  archiveKnowledgeIndex,
  createKnowledgeIndex,
  getKnowledgeIndex,
  listKnowledgeIndexDocuments,
  listKnowledgeIndexes,
  syncKnowledgeIndexNow,
  updateKnowledgeIndex,
} from "@/lib/api/knowledge-indexes";
import type {
  CreateKnowledgeIndexRequest,
  KnowledgeIndex,
  ListKnowledgeIndexesParams,
  UpdateKnowledgeIndexRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

function syncIndexCache(queryClient: QueryClient, index: KnowledgeIndex) {
  queryClient.setQueriesData<KnowledgeIndex | undefined>(
    { queryKey: queryKeys.knowledgeIndexes.detail(index.id) },
    () => index,
  );
  queryClient.setQueriesData<KnowledgeIndex[] | undefined>(
    { queryKey: queryKeys.knowledgeIndexes.all },
    (existing) =>
      existing?.map((candidate) => (candidate.id === index.id ? index : candidate)) ?? existing,
  );
}

function invalidateIndexQueries(queryClient: QueryClient, indexId?: string) {
  queryClient.invalidateQueries({ queryKey: queryKeys.knowledgeIndexes.all });
  if (indexId) {
    queryClient.invalidateQueries({ queryKey: queryKeys.knowledgeIndexes.detail(indexId) });
    queryClient.invalidateQueries({ queryKey: queryKeys.knowledgeIndexes.documents(indexId) });
  }
}

export function useKnowledgeIndexes(
  options: ListKnowledgeIndexesParams & { enabled?: boolean } = {},
) {
  const includeArchived = options.includeArchived ?? false;
  const search = options.search?.trim() ?? "";

  return useOrgScopedQuery({
    queryKey: queryKeys.knowledgeIndexes.list(includeArchived, search),
    queryFn: () => listKnowledgeIndexes({ includeArchived, search }),
    enabled: options.enabled ?? true,
  });
}

export function useKnowledgeIndex(indexId: string | undefined) {
  const query = useOrgScopedQuery({
    queryKey: queryKeys.knowledgeIndexes.detail(indexId ?? ""),
    queryFn: () => getKnowledgeIndex(indexId!),
    enabled: !!indexId,
  });
  const fallback = useResourceOrgFallback({
    resourceId: indexId,
    error: query.error,
    isLoading: query.isLoading,
  });
  return {
    ...query,
    isLoading: query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useKnowledgeIndexDocuments(indexId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.knowledgeIndexes.documents(indexId ?? ""),
    queryFn: () => listKnowledgeIndexDocuments(indexId!),
    enabled: !!indexId,
  });
}

export function useCreateKnowledgeIndex() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateKnowledgeIndexRequest) => createKnowledgeIndex(request),
    onSuccess: (index) => {
      syncIndexCache(queryClient, index);
      invalidateIndexQueries(queryClient, index.id);
    },
  });
}

export function useUpdateKnowledgeIndex() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ indexId, data }: { indexId: string; data: UpdateKnowledgeIndexRequest }) =>
      updateKnowledgeIndex(indexId, data),
    onSuccess: (index) => {
      syncIndexCache(queryClient, index);
      invalidateIndexQueries(queryClient, index.id);
    },
  });
}

export function useSyncKnowledgeIndex() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (indexId: string) => syncKnowledgeIndexNow(indexId),
    onSuccess: (index) => {
      syncIndexCache(queryClient, index);
      invalidateIndexQueries(queryClient, index.id);
    },
  });
}

export function useArchiveKnowledgeIndex() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (indexId: string) => archiveKnowledgeIndex(indexId),
    onSuccess: (_result, indexId) => {
      invalidateIndexQueries(queryClient, indexId);
    },
  });
}
