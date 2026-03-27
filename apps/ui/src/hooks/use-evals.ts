// Eval hooks
"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  evalsCrudApi,
  listEvalCases,
  createEvalCase,
  deleteEvalCase,
  listEvalRuns,
  getEvalRun,
  createEvalRun,
  cancelEvalRun,
} from "@/lib/api/evals";
import type {
  CreateEvalRequest,
  UpdateEvalRequest,
  CreateEvalCaseRequest,
  CreateEvalRunRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";
import { useOrgScopedQuery } from "./create-crud-hooks";

const evalCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof evalsCrudApi.get>>,
  CreateEvalRequest,
  UpdateEvalRequest
>({
  api: evalsCrudApi,
  queryKeys: queryKeys.evals,
});

export const useEvals = evalCrudHooks.useList;
export const useEval = evalCrudHooks.useDetail;
export const useCreateEval = evalCrudHooks.useCreate;
export const useDeleteEval = evalCrudHooks.useDelete;
export const useDestroyEval = evalCrudHooks.useDestroy;

export function useUpdateEval() {
  const mutation = evalCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { evalId: string; request: UpdateEvalRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.evalId, request: variables.request }, options),
    mutateAsync: (
      variables: { evalId: string; request: UpdateEvalRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.evalId, request: variables.request }, options),
  };
}

// Eval cases hooks
export function useEvalCases(evalId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.cases(evalId ?? ""),
    queryFn: () => listEvalCases(evalId!),
    enabled: !!evalId,
  });
}

export function useCreateEvalCase(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateEvalCaseRequest) => createEvalCase(evalId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.cases(evalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.detail(evalId) });
    },
  });
}

export function useDeleteEvalCase(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (caseId: string) => deleteEvalCase(evalId, caseId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.cases(evalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.detail(evalId) });
    },
  });
}

// Eval runs hooks
export function useEvalRuns(evalId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.runs(evalId ?? ""),
    queryFn: () => listEvalRuns(evalId!),
    enabled: !!evalId,
  });
}

export function useEvalRun(evalId: string | undefined, runId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.runDetail(evalId ?? "", runId ?? ""),
    queryFn: () => getEvalRun(evalId!, runId!),
    enabled: !!evalId && !!runId,
  });
}

export function useCreateEvalRun(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateEvalRunRequest) => createEvalRun(evalId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.runs(evalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.detail(evalId) });
    },
  });
}

export function useCancelEvalRun(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runId: string) => cancelEvalRun(evalId, runId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.runs(evalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.detail(evalId) });
    },
  });
}
