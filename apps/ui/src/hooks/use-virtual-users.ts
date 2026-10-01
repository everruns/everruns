"use client";

import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createVirtualUser,
  deleteVirtualUser,
  destroyVirtualUser,
  getVirtualUser,
  listVirtualUsers,
  updateVirtualUser,
} from "@/lib/api/virtual-users";
import { useOrg } from "@/providers/org-provider";
import type { CreateVirtualUserRequest, UpdateVirtualUserRequest } from "@/lib/api/types";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

const virtualUserKeys = {
  all: ["virtual-users"] as const,
  list: (includeArchived: boolean) => ["virtual-users", "list", includeArchived] as const,
  detail: (id: string) => ["virtual-users", "detail", id] as const,
};

export function useVirtualUsers(
  options: {
    includeArchived?: boolean;
    enabled?: boolean;
    usage?: "end_user" | "service";
    search?: string;
  } = {},
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const includeArchived = options.includeArchived ?? false;
  const org = currentOrg?.public_id;
  const query = useInfiniteQuery({
    queryKey: [...virtualUserKeys.list(includeArchived), org, options.usage, options.search],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      listVirtualUsers(includeArchived, {
        offset: pageParam,
        usage: options.usage,
        search: options.search,
      }),
    getNextPageParam: (lastPage) =>
      lastPage.offset + lastPage.data.length < lastPage.total
        ? lastPage.offset + lastPage.data.length
        : undefined,
    enabled: !!org && (options.enabled ?? true),
  });
  return {
    ...query,
    data: query.data?.pages.flatMap((page) => page.data),
    total: query.data?.pages[0]?.total,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useVirtualUser(identityId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...virtualUserKeys.detail(identityId ?? ""), org],
    queryFn: () => getVirtualUser(identityId!),
    enabled: !!org && !!identityId,
  });
  const fallback = useResourceOrgFallback({
    resourceId: identityId,
    error: query.error,
    isLoading: orgLoading || query.isLoading,
  });
  return {
    ...query,
    isLoading: orgLoading || query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useCreateVirtualUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateVirtualUserRequest) => createVirtualUser(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: virtualUserKeys.all });
    },
  });
}

export function useUpdateVirtualUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      identityId,
      request,
    }: {
      identityId: string;
      request: UpdateVirtualUserRequest;
    }) => updateVirtualUser(identityId, request),
    onSuccess: (identity) => {
      queryClient.invalidateQueries({ queryKey: virtualUserKeys.all });
      queryClient.setQueryData(virtualUserKeys.detail(identity.id), identity);
    },
  });
}

export function useDeleteVirtualUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (identityId: string) => deleteVirtualUser(identityId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: virtualUserKeys.all });
    },
  });
}

export function useDestroyVirtualUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (identityId: string) => destroyVirtualUser(identityId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: virtualUserKeys.all });
    },
  });
}
