// Harness hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  copyHarness,
  createHarness,
  deleteHarness,
  getHarness,
  listHarnesses,
  updateHarness,
} from "@/lib/api/harnesses";
import { queryKeys } from "@/lib/query-keys";
import type { CreateHarnessRequest, UpdateHarnessRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export function useHarnesses() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.harnesses.list(), org],
    queryFn: () => listHarnesses(),
    enabled: !!org,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useHarness(harnessId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.harnesses.detail(harnessId!), org],
    queryFn: () => getHarness(harnessId!),
    enabled: !!org && !!harnessId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateHarnessRequest) => createHarness(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
    },
  });
}

export function useUpdateHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ harnessId, request }: { harnessId: string; request: UpdateHarnessRequest }) =>
      updateHarness(harnessId, request),
    onSuccess: (_, { harnessId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.harnesses.detail(harnessId),
      });
    },
  });
}

export function useCopyHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (harnessId: string) => copyHarness(harnessId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
    },
  });
}

export function useDeleteHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (harnessId: string) => deleteHarness(harnessId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
    },
  });
}
