"use client";

import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  archiveVolume,
  createVolume,
  getVolume,
  listVolumes,
  syncVolumeNow,
  updateVolume,
} from "@/lib/api/volumes";
import type {
  CreateVolumeRequest,
  ListVolumesParams,
  UpdateVolumeRequest,
  Volume,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

function syncVolumeCache(queryClient: QueryClient, volume: Volume) {
  queryClient.setQueriesData<Volume | undefined>(
    { queryKey: queryKeys.volumes.detail(volume.id) },
    () => volume,
  );
  queryClient.setQueriesData<Volume[] | undefined>(
    { queryKey: queryKeys.volumes.all },
    (existing) =>
      existing?.map((candidate) => (candidate.id === volume.id ? volume : candidate)) ?? existing,
  );
}

function invalidateVolumeQueries(queryClient: QueryClient, volumeId?: string) {
  queryClient.invalidateQueries({ queryKey: queryKeys.volumes.all });
  if (volumeId) {
    queryClient.invalidateQueries({ queryKey: queryKeys.volumes.detail(volumeId) });
  }
}

export function useVolumes(options: ListVolumesParams = {}) {
  const includeArchived = options.includeArchived ?? false;
  const search = options.search?.trim() ?? "";

  return useOrgScopedQuery({
    queryKey: queryKeys.volumes.list(includeArchived, search),
    queryFn: () => listVolumes({ includeArchived, search }),
  });
}

export function useVolume(volumeId: string | undefined) {
  const query = useOrgScopedQuery({
    queryKey: queryKeys.volumes.detail(volumeId ?? ""),
    queryFn: () => getVolume(volumeId!),
    enabled: !!volumeId,
  });
  const fallback = useResourceOrgFallback({
    resourceId: volumeId,
    error: query.error,
    isLoading: query.isLoading,
  });
  return {
    ...query,
    isLoading: query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useCreateVolume() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateVolumeRequest) => createVolume(request),
    onSuccess: (volume) => {
      syncVolumeCache(queryClient, volume);
      invalidateVolumeQueries(queryClient, volume.id);
    },
  });
}

export function useUpdateVolume() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ volumeId, data }: { volumeId: string; data: UpdateVolumeRequest }) =>
      updateVolume(volumeId, data),
    onSuccess: (volume) => {
      syncVolumeCache(queryClient, volume);
      invalidateVolumeQueries(queryClient, volume.id);
    },
  });
}

export function useSyncVolume() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (volumeId: string) => syncVolumeNow(volumeId),
    onSuccess: (volume) => {
      syncVolumeCache(queryClient, volume);
      invalidateVolumeQueries(queryClient, volume.id);
    },
  });
}

export function useArchiveVolume() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (volumeId: string) => archiveVolume(volumeId),
    onSuccess: (_result, volumeId) => {
      invalidateVolumeQueries(queryClient, volumeId);
    },
  });
}
