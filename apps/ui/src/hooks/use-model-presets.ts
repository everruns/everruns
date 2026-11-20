"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  applyModelPreset,
  createModelPreset,
  deleteModelPreset,
  listModelPresets,
  updateModelPreset,
  type ModelPresetJobs,
} from "@/lib/api/model-presets";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

const KEY = ["model-presets"] as const;

export function useModelPresets(options: { enabled?: boolean } = {}) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: [...KEY, currentOrg?.public_id],
    queryFn: listModelPresets,
    enabled: !!currentOrg && options.enabled !== false,
    staleTime: 30000,
  });
}

export function useModelPresetMutations() {
  const client = useQueryClient();
  const refresh = () => client.invalidateQueries({ queryKey: KEY });
  return {
    create: useMutation({
      mutationFn: createModelPreset,
      onSuccess: refresh,
    }),
    update: useMutation({
      mutationFn: ({
        id,
        ...request
      }: ModelPresetJobs & { id: string; name?: string; description?: string | null }) =>
        updateModelPreset(id, request),
      onSuccess: refresh,
    }),
    remove: useMutation({ mutationFn: deleteModelPreset, onSuccess: refresh }),
    apply: useMutation({
      mutationFn: applyModelPreset,
      onSuccess: () =>
        Promise.all([
          refresh(),
          client.invalidateQueries({ queryKey: queryKeys.organizations.all }),
          client.invalidateQueries({ queryKey: ["decision-default"] }),
        ]),
    }),
  };
}
