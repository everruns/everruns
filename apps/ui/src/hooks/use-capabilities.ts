// Capability hooks
//
// Note: Agent-specific capabilities are now part of the agent resource.
// Use useAgent() to get an agent with its capabilities included.
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createDeclarativeCapability,
  deleteDeclarativeCapability,
  getCapability,
  getDeclarativeCapability,
  listCapabilities,
  listDeclarativeCapabilities,
  updateDeclarativeCapability,
} from "@/lib/api/capabilities";
import type {
  CapabilityId,
  CreateDeclarativeCapabilityRequest,
  UpdateDeclarativeCapabilityRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

export function useCapabilities(options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: ["capabilities", org],
    queryFn: () => listCapabilities(),
    enabled: !!org && (options.enabled ?? true),
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCapability(capabilityId: CapabilityId | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: ["capability", capabilityId, org],
    queryFn: () => getCapability(capabilityId!),
    enabled: !!org && !!capabilityId,
  });

  const fallback = useResourceOrgFallback({
    resourceId: capabilityId,
    error: query.error,
    isLoading: orgLoading || query.isLoading,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useDeclarativeCapabilities(options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.declarativeCapabilities.list(), org],
    queryFn: () => listDeclarativeCapabilities(),
    enabled: !!org && (options.enabled ?? true),
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useDeclarativeCapability(id: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.declarativeCapabilities.detail(id ?? ""), org],
    queryFn: () => getDeclarativeCapability(id!),
    enabled: !!org && !!id,
  });

  const fallback = useResourceOrgFallback({
    resourceId: id,
    error: query.error,
    isLoading: orgLoading || query.isLoading,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useCreateDeclarativeCapability() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateDeclarativeCapabilityRequest) =>
      createDeclarativeCapability(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.capabilities.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.declarativeCapabilities.all });
    },
  });
}

export function useUpdateDeclarativeCapability() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateDeclarativeCapabilityRequest }) =>
      updateDeclarativeCapability(id, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.capabilities.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.declarativeCapabilities.all });
    },
  });
}

export function useDeleteDeclarativeCapability() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteDeclarativeCapability(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.capabilities.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.declarativeCapabilities.all });
    },
  });
}
