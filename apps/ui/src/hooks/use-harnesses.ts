// Harness hooks (M2)

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createHarness,
  deleteHarness,
  getHarness,
  listHarnesses,
  updateHarness,
} from "@/lib/api/harnesses";
import type { CreateHarnessRequest, UpdateHarnessRequest } from "@/lib/api/types";

export function useHarnesses() {
  return useQuery({
    queryKey: ["harnesses"],
    queryFn: listHarnesses,
  });
}

export function useHarness(harnessId: string | undefined) {
  return useQuery({
    queryKey: ["harness", harnessId],
    queryFn: () => getHarness(harnessId!),
    enabled: !!harnessId,
  });
}

export function useCreateHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateHarnessRequest) => createHarness(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["harnesses"] });
    },
  });
}

export function useUpdateHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      harnessId,
      request,
    }: {
      harnessId: string;
      request: UpdateHarnessRequest;
    }) => updateHarness(harnessId, request),
    onSuccess: (_, { harnessId }) => {
      queryClient.invalidateQueries({ queryKey: ["harnesses"] });
      queryClient.invalidateQueries({ queryKey: ["harness", harnessId] });
    },
  });
}

export function useDeleteHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (harnessId: string) => deleteHarness(harnessId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["harnesses"] });
    },
  });
}
