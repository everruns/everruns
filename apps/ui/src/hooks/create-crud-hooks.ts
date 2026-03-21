"use client";

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from "@tanstack/react-query";
import type { CrudApi } from "@/lib/api/crud";
import { useOrg } from "@/providers/org-provider";

interface UseCrudListOptions {
  includeArchived?: boolean;
}

type QueryKeyValue = readonly unknown[];

interface CrudQueryKeys {
  all: QueryKeyValue;
  list(includeArchived?: boolean): QueryKeyValue;
  detail(id: string): QueryKeyValue;
}

interface UseOrgScopedQueryOptions<TData> {
  queryKey: QueryKeyValue;
  queryFn: () => Promise<TData>;
  enabled?: boolean;
  staleTime?: number;
}

export function useOrgScopedQuery<TData>({
  queryKey,
  queryFn,
  enabled = true,
  staleTime,
}: UseOrgScopedQueryOptions<TData>): UseQueryResult<TData> & { isLoading: boolean } {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKey, org],
    queryFn,
    enabled: enabled && !!org,
    staleTime,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  } as UseQueryResult<TData> & { isLoading: boolean };
}

function invalidateCrudQueries(queryClient: QueryClient, queryKeys: CrudQueryKeys, id?: string) {
  queryClient.invalidateQueries({ queryKey: queryKeys.all });
  if (id) {
    queryClient.invalidateQueries({ queryKey: queryKeys.detail(id) });
  }
}

interface CreateCrudHooksOptions<TItem, TCreate, TUpdate> {
  api: CrudApi<TItem, TCreate, TUpdate>;
  queryKeys: CrudQueryKeys;
  staleTime?: number;
  syncEntityCache?(queryClient: QueryClient, item: TItem): void;
}

interface UpdateMutationVariables<TUpdate> {
  id: string;
  request: TUpdate;
}

export function createCrudHooks<TItem, TCreate, TUpdate>({
  api,
  queryKeys,
  staleTime,
  syncEntityCache,
}: CreateCrudHooksOptions<TItem, TCreate, TUpdate>) {
  function useList(options: UseCrudListOptions = {}) {
    const includeArchived = options.includeArchived ?? false;

    return useOrgScopedQuery({
      queryKey: queryKeys.list(includeArchived),
      queryFn: () => api.list(includeArchived),
      staleTime,
    });
  }

  function useDetail(id: string | undefined) {
    return useOrgScopedQuery({
      queryKey: queryKeys.detail(id ?? ""),
      queryFn: () => api.get(id!),
      enabled: !!id,
      staleTime,
    });
  }

  function useCreate(): UseMutationResult<TItem, Error, TCreate> {
    const queryClient = useQueryClient();

    return useMutation({
      mutationFn: api.create,
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: queryKeys.all });
      },
    });
  }

  function useUpdate(): UseMutationResult<TItem, Error, UpdateMutationVariables<TUpdate>> {
    const queryClient = useQueryClient();

    return useMutation({
      mutationFn: ({ id, request }) => api.update(id, request),
      onSuccess: (item, { id }) => {
        syncEntityCache?.(queryClient, item);
        invalidateCrudQueries(queryClient, queryKeys, id);
      },
    });
  }

  function useDelete(): UseMutationResult<void, Error, string> {
    const queryClient = useQueryClient();

    return useMutation({
      mutationFn: api.delete,
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: queryKeys.all });
      },
    });
  }

  function useDestroy(): UseMutationResult<void, Error, string> {
    const queryClient = useQueryClient();

    return useMutation({
      mutationFn: api.destroy,
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: queryKeys.all });
      },
    });
  }

  return {
    useList,
    useDetail,
    useCreate,
    useUpdate,
    useDelete,
    useDestroy,
  };
}
