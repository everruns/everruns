// Eval hooks
"use client";

import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import {
  evalsCrudApi,
  listEvalCases,
  createEvalCase,
  updateEvalCase,
  deleteEvalCase,
  listEvalRuns,
  getEvalRun,
  createEvalRun,
  cancelEvalRun,
  listEvalExperiments,
  listAllEvalExperiments,
  getEvalExperiment,
  createEvalExperiment,
  listEvalProposals,
  proposeEvalCases,
  acceptEvalProposal,
  dismissEvalProposal,
  getFindingFix,
  checkFindingFix,
  updateFindingFix,
  applyFindingFix,
  rejectFindingFix,
} from "@/lib/api/evals";
import type {
  CreateEvalRequest,
  UpdateEvalRequest,
  CreateEvalCaseRequest,
  UpdateEvalCaseRequest,
  CreateEvalRunRequest,
  EvalRun,
  EvalExperiment,
  CreateEvalExperimentRequest,
} from "@/lib/api/types";
import { MAX_COMPARE_RUNS } from "@/lib/eval-run-comparison";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
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
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.cases(evalId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
    },
  });
}

export function useUpdateEvalCase(evalId: string, caseId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: UpdateEvalCaseRequest) => updateEvalCase(evalId, caseId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.cases(evalId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
    },
  });
}

export function useDeleteEvalCase(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (caseId: string) => deleteEvalCase(evalId, caseId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.cases(evalId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
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

// Fetch several runs (with their case results) at once for cross-run
// comparison. Org-scoped like `useEvalRun`; reuses the per-run query cache so a
// run already viewed isn't refetched.
export function useEvalRunComparison(evalId: string | undefined, runIds: string[]) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const boundedRunIds = runIds.slice(0, MAX_COMPARE_RUNS);

  const queries = useQueries({
    queries: boundedRunIds.map((runId) => ({
      queryKey: [...queryKeys.evals.runDetail(evalId ?? "", runId), org],
      queryFn: () => getEvalRun(evalId!, runId),
      enabled: !!evalId && !!org && !!runId,
    })),
  });

  return {
    runs: queries.map((q) => q.data).filter((d): d is EvalRun => !!d),
    isLoading: orgLoading || queries.some((q) => q.isLoading),
    isError: queries.some((q) => q.isError),
    errors: queries.map((q) => q.error).filter((error): error is Error => error instanceof Error),
  };
}

export function useCreateEvalRun(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateEvalRunRequest) => createEvalRun(evalId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.runs(evalId) });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
    },
  });
}

export function useCancelEvalRun(evalId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runId: string) => cancelEvalRun(evalId, runId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.runs(evalId) });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
    },
  });
}

// Experiments. A running experiment polls until every variant's run finished.
const EXPERIMENT_POLL_MS = 3_000;

export function useEvalExperiments(evalId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.experiments(evalId ?? ""),
    queryFn: () => listEvalExperiments(evalId!),
    enabled: !!evalId,
  });
}

export function useAllEvalExperiments(enabled = true) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.allExperiments,
    queryFn: listAllEvalExperiments,
    enabled,
  });
}

export function useEvalExperiment(evalId: string | undefined, experimentId: string | undefined) {
  return useOrgScopedQuery<EvalExperiment>({
    queryKey: queryKeys.evals.experimentDetail(evalId ?? "", experimentId ?? ""),
    queryFn: () => getEvalExperiment(evalId!, experimentId!),
    enabled: !!evalId && !!experimentId,
    refetchInterval: (query) =>
      query.state.data?.status === "running" ? EXPERIMENT_POLL_MS : false,
  });
}

export function useCreateEvalExperiment() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ evalId, request }: { evalId: string; request: CreateEvalExperimentRequest }) =>
      createEvalExperiment(evalId, request),
    onSuccess: (_experiment, { evalId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.experiments(evalId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.allExperiments,
      });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.runs(evalId) });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
    },
  });
}

// Runs of several datasets at once, for trend lines on the Evals overview and
// an agent's Evals tab. Bounded so a large org does not fan out unboundedly.
const MAX_TREND_EVALS = 20;

export function useEvalRunsMany(evalIds: string[]): Map<string, EvalRun[]> {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  const ids = evalIds.slice(0, MAX_TREND_EVALS);
  const queries = useQueries({
    queries: ids.map((evalId) => ({
      queryKey: [...queryKeys.evals.runs(evalId), org],
      queryFn: () => listEvalRuns(evalId),
      enabled: !!org,
    })),
  });
  return new Map(ids.map((evalId, index) => [evalId, queries[index]?.data ?? []]));
}

// Eval Builder proposals for an agent. While a run is going the list polls,
// so its proposals appear without a reload.
const PROPOSALS_POLL_MS = 4_000;

export function useEvalProposals(agentId: string | undefined, enabled = true) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.proposals(agentId ?? ""),
    queryFn: () => listEvalProposals(agentId!),
    enabled: !!agentId && enabled,
    refetchInterval: (query) =>
      query.state.data?.latest_run?.status === "running" ? PROPOSALS_POLL_MS : false,
  });
}

export function useProposeEvalCases(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => proposeEvalCases(agentId),
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.proposals(agentId),
      }),
  });
}

export function useAcceptEvalProposal(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      proposalId,
      evalId,
      reason,
    }: {
      proposalId: string;
      evalId: string;
      reason?: string;
    }) => acceptEvalProposal(proposalId, evalId, reason),
    onSuccess: (_case, { evalId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.proposals(agentId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.cases(evalId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.detail(evalId),
      });
      queryClient.invalidateQueries({ queryKey: queryKeys.evals.all });
    },
  });
}

export function useDismissEvalProposal(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ proposalId, note }: { proposalId: string; note?: string }) =>
      dismissEvalProposal(proposalId, note),
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: queryKeys.evals.proposals(agentId),
      }),
  });
}

// A finding's suggested fix (knowledge/evaluation/suggested-fixes.md). Polled
// while its check runs, so the result appears without a reload.
const FIX_CHECK_POLL_MS = 10_000;

export function useFindingFix(issueId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.evals.findingFix(issueId ?? ""),
    queryFn: () => getFindingFix(issueId!),
    enabled: !!issueId,
    refetchInterval: (query) =>
      query.state.data?.status === "checking" ? FIX_CHECK_POLL_MS : false,
  });
}

export function useFindingFixAction(issueId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (
      action:
        | { kind: "check"; fixId: string }
        | { kind: "edit"; fixId: string; addition: string }
        | { kind: "apply"; fixId: string; reason?: string }
        | { kind: "reject"; fixId: string; note: string },
    ) => {
      switch (action.kind) {
        case "check":
          return checkFindingFix(action.fixId);
        case "edit":
          return updateFindingFix(action.fixId, action.addition);
        case "apply":
          return applyFindingFix(action.fixId, action.reason);
        case "reject":
          return rejectFindingFix(action.fixId, action.note);
      }
    },
    onSuccess: (fix) => {
      queryClient.setQueryData(queryKeys.evals.findingFix(issueId), fix);
      queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
  });
}
