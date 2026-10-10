"use client";

import type { ModelWithProvider } from "@/lib/api/types";
import { api } from "@/lib/api/client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getProviders,
  getProvider,
  getProvidersConfig,
  createProvider,
  updateProvider,
  deleteProvider,
  syncProviderModels,
  reviewProviderModels,
  getModels,
  getProviderModels,
  getModel,
  createModel,
  updateModel,
  deleteModel,
} from "@/lib/api/providers";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateProviderRequest,
  UpdateProviderRequest,
  CreateModelRequest,
  UpdateModelRequest,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Provider hooks
export function useProviders() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.list(), org],
    queryFn: () => getProviders(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useProvider(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.detail(providerId), org],
    queryFn: () => getProvider(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

// Per-driver credential schemas (and caller policies), used to render the
// provider credential forms as discrete typed inputs.
export function useProvidersConfig(options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.all, "config", org],
    queryFn: () => getProvidersConfig(),
    enabled: !!org && (options.enabled ?? true),
    staleTime: 300000,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateProviderRequest) => createProvider(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      // A provider created with a credential also discovers its models and may
      // elect the org default model server-side, so the model caches are stale
      // the moment this resolves.
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
    },
  });
}

export function useUpdateProvider(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateProviderRequest) => updateProvider(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.providers.detail(providerId),
      });
      // Same as create: a key landing on an existing provider discovers models
      // and may elect the org default.
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
    },
  });
}

export function useDeleteProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => deleteProvider(providerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
    },
  });
}

export function useSyncProviderModels() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => syncProviderModels(providerId),
    onSuccess: () => {
      // Refresh models list after sync. A sync may also elect the org default
      // model when none resolved, so org settings are stale too.
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
    },
  });
}

// Model hooks
export function useModels(options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.models.list(), org],
    queryFn: () => getModels(),
    enabled: !!org && (options.enabled ?? true),
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useProviderModels(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.models(providerId), org],
    queryFn: () => getProviderModels(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useModel(modelId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.models.detail(modelId), org],
    queryFn: () => getModel(modelId),
    enabled: !!org && !!modelId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateModel(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateModelRequest) => createModel(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.providers.models(providerId),
      });
    },
  });
}

export function useUpdateModel(modelId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateModelRequest) => updateModel(modelId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
    },
  });
}

export function useDeleteModel() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (modelId: string) => deleteModel(modelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
    },
  });
}

/** Resolve the server default for a draft that has no persisted session. */
export function useDefaultModel() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...queryKeys.models.list(), "default", org],
    queryFn: async () => (await api.get<ModelWithProvider | null>("/v1/models/default")).data,
    enabled: !!org,
    staleTime: 30_000,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}

export function useModelProfiles(
  providerId?: string,
  service?: import("@/lib/api/types").ModelService,
) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["model-profiles", currentOrg?.public_id, providerId, service],
    enabled: !!currentOrg && !!providerId,
    queryFn: async () => {
      const params = new URLSearchParams();
      if (providerId) params.set("provider_id", providerId);
      if (service) params.set("service", service);
      return (
        await api.get<
          import("@/lib/api/types").ListResponse<import("@/lib/api/types").ModelProfileResponse>
        >(`/v1/model-profiles?${params}`)
      ).data.data;
    },
  });
}
export function useDecisionDefault() {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  const query = useQuery({
    queryKey: ["decision-default", currentOrg?.public_id],
    enabled: !!currentOrg,
    queryFn: async () =>
      (await api.get<ModelWithProvider | null>("/v1/models/decision-default")).data,
  });
  const mutation = useMutation({
    mutationFn: async (model_id: string | null) =>
      (await api.put("/v1/models/decision-default", { model_id })).data,
    onSuccess: () => client.invalidateQueries({ queryKey: ["decision-default"] }),
  });
  return { ...query, setDefault: mutation };
}

/** Mark providers' discovered models as reviewed (clears `is_new`). */
export function useReviewProviderModels() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (providerIds: string[]) => {
      await Promise.all(providerIds.map((id) => reviewProviderModels(id)));
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
    },
  });
}

// PATCHes run a few at a time: a catalog can hold hundreds of models and there
// is no bulk endpoint, so an unbounded fan-out would trip rate limits.
const BULK_CONCURRENCY = 8;

/**
 * Enable and disable many models in one action. Resolves with the ids that
 * failed so the caller can report a partial result instead of a blanket error.
 */
export function useSetModelsEnabled() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (changes: { id: string; enabled: boolean }[]) => {
      const failed: string[] = [];
      for (let start = 0; start < changes.length; start += BULK_CONCURRENCY) {
        const batch = changes.slice(start, start + BULK_CONCURRENCY);
        const results = await Promise.allSettled(
          batch.map((change) => updateModel(change.id, { enabled: change.enabled })),
        );
        results.forEach((result, index) => {
          if (result.status === "rejected") failed.push(batch[index].id);
        });
      }
      return { failed };
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      // Disabling the org default clears it server-side.
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
      queryClient.invalidateQueries({ queryKey: ["decision-default"] });
    },
  });
}
