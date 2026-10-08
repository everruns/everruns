"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import {
  createOrganizationConnection,
  deleteOrganizationConnection,
  listOrganizationConnections,
  updateOrganizationConnection,
  verifyOrganizationConnection,
  type SaveOrganizationConnection,
} from "@/lib/api/organization-connections";

const key = (org?: string) => ["organization-connections", org];

export function useOrganizationConnections() {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: key(currentOrg?.public_id),
    enabled: !!currentOrg,
    queryFn: () => listOrganizationConnections(currentOrg!.public_id),
  });
}

export function useSaveOrganizationConnection() {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      provider,
      id,
      input,
    }: {
      provider: string;
      id?: string;
      input: SaveOrganizationConnection;
    }) =>
      id
        ? updateOrganizationConnection(currentOrg!.public_id, id, input)
        : createOrganizationConnection(currentOrg!.public_id, provider, input),
    onSuccess: () => client.invalidateQueries({ queryKey: key(currentOrg?.public_id) }),
  });
}

export function useDeleteOrganizationConnection() {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteOrganizationConnection(currentOrg!.public_id, id),
    onSuccess: () => client.invalidateQueries({ queryKey: key(currentOrg?.public_id) }),
  });
}

export function useVerifyOrganizationConnection() {
  const { currentOrg } = useOrg();
  return useMutation({
    mutationFn: (id: string) => verifyOrganizationConnection(currentOrg!.public_id, id),
  });
}
