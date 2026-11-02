// Eval API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  Eval,
  CreateEvalRequest,
  UpdateEvalRequest,
  EvalCase,
  CreateEvalCaseRequest,
  UpdateEvalCaseRequest,
  EvalRun,
  CreateEvalRunRequest,
  EvalRunShareLink,
  EvalRunShareStatus,
  PublicEvalRun,
  EvalExperiment,
  CreateEvalExperimentRequest,
  EvalBuilderRun,
  EvalCaseProposal,
  EvalProposalList,
} from "./types";
import { changeReasonOptions } from "./change-history";

export const evalsCrudApi = createCrudApi<Eval, CreateEvalRequest, UpdateEvalRequest>("/v1/evals");

export const createEval = evalsCrudApi.create;
export const listEvals = evalsCrudApi.list;
export const getEval = evalsCrudApi.get;
export const updateEval = evalsCrudApi.update;
export const deleteEval = evalsCrudApi.delete;
export const destroyEval = evalsCrudApi.destroy;

function evalPath(evalId: string): string {
  return `/v1/evals/${encodeURIComponent(evalId)}`;
}

function evalRunPath(evalId: string, runId: string): string {
  return `${evalPath(evalId)}/runs/${encodeURIComponent(runId)}`;
}

// Eval cases (nested under eval)
export async function listEvalCases(evalId: string): Promise<EvalCase[]> {
  const response = await api.get<{ data: EvalCase[] }>(`${evalPath(evalId)}/cases`);
  return response.data.data;
}

export async function createEvalCase(
  evalId: string,
  request: CreateEvalCaseRequest,
): Promise<EvalCase> {
  const response = await api.post<EvalCase>(`${evalPath(evalId)}/cases`, request);
  return response.data;
}

export async function updateEvalCase(
  evalId: string,
  caseId: string,
  request: UpdateEvalCaseRequest,
): Promise<EvalCase> {
  const response = await api.patch<EvalCase>(
    `${evalPath(evalId)}/cases/${encodeURIComponent(caseId)}`,
    request,
  );
  return response.data;
}

export async function deleteEvalCase(evalId: string, caseId: string): Promise<void> {
  await api.delete(`${evalPath(evalId)}/cases/${encodeURIComponent(caseId)}`);
}

// Eval runs (nested under eval)
export async function listEvalRuns(evalId: string): Promise<EvalRun[]> {
  const response = await api.get<{ data: EvalRun[] }>(`${evalPath(evalId)}/runs`);
  return response.data.data;
}

export async function getEvalRun(evalId: string, runId: string): Promise<EvalRun> {
  const response = await api.get<EvalRun>(evalRunPath(evalId, runId));
  return response.data;
}

export async function createEvalRun(
  evalId: string,
  request: CreateEvalRunRequest,
): Promise<EvalRun> {
  const response = await api.post<EvalRun>(`${evalPath(evalId)}/runs`, request);
  return response.data;
}

export async function cancelEvalRun(evalId: string, runId: string): Promise<void> {
  await api.post(`${evalRunPath(evalId, runId)}/cancel`, {});
}

// Read-only share links for a run.
export async function getRunShareStatus(
  evalId: string,
  runId: string,
): Promise<EvalRunShareStatus> {
  const response = await api.get<EvalRunShareStatus>(`${evalRunPath(evalId, runId)}/share`);
  return response.data;
}

export async function createRunShare(evalId: string, runId: string): Promise<EvalRunShareLink> {
  const response = await api.post<EvalRunShareLink>(`${evalRunPath(evalId, runId)}/share`, {});
  return response.data;
}

export async function revokeRunShare(evalId: string, runId: string): Promise<void> {
  await api.delete(`${evalRunPath(evalId, runId)}/share`);
}

// Public (unauthenticated) read of a shared run. Uses a bare fetch — no auth
// cookies/headers needed — so it works for anonymous viewers.
export async function getPublicEvalRun(token: string): Promise<PublicEvalRun> {
  const res = await fetch(`/api/v1/public/eval-runs/${encodeURIComponent(token)}`, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new Error(res.status === 404 ? "not_found" : `request failed (${res.status})`);
  }
  return (await res.json()) as PublicEvalRun;
}

// Experiments: one dataset version against several variants (Evals rebuild
// step 3). Each variant is an ordinary run tagged with the experiment.
export async function listEvalExperiments(evalId: string): Promise<EvalExperiment[]> {
  const response = await api.get<{ data: EvalExperiment[] }>(`${evalPath(evalId)}/experiments`);
  return response.data.data;
}

export async function listAllEvalExperiments(): Promise<EvalExperiment[]> {
  const response = await api.get<{ data: EvalExperiment[] }>(`/v1/evals/experiments`);
  return response.data.data;
}

export async function getEvalExperiment(
  evalId: string,
  experimentId: string,
): Promise<EvalExperiment> {
  const response = await api.get<EvalExperiment>(
    `${evalPath(evalId)}/experiments/${encodeURIComponent(experimentId)}`,
  );
  return response.data;
}

export async function createEvalExperiment(
  evalId: string,
  request: CreateEvalExperimentRequest,
): Promise<EvalExperiment> {
  const response = await api.post<EvalExperiment>(`${evalPath(evalId)}/experiments`, request);
  return response.data;
}

// Eval Builder (Evals rebuild step 8): proposed cases for an agent's
// datasets. Nothing becomes a case until a person accepts it.
function proposalsPath(agentId: string): string {
  return `/v1/agents/${encodeURIComponent(agentId)}/eval-proposals`;
}

export async function listEvalProposals(agentId: string): Promise<EvalProposalList> {
  const response = await api.get<EvalProposalList>(proposalsPath(agentId));
  return response.data;
}

export async function proposeEvalCases(agentId: string): Promise<EvalBuilderRun> {
  const response = await api.post<EvalBuilderRun>(proposalsPath(agentId));
  return response.data;
}

export async function acceptEvalProposal(
  proposalId: string,
  evalId: string,
  reason?: string,
): Promise<EvalCase> {
  const response = await api.post<EvalCase>(
    `/v1/eval-proposals/${encodeURIComponent(proposalId)}/accept`,
    { eval_id: evalId },
    changeReasonOptions(reason),
  );
  return response.data;
}

export async function dismissEvalProposal(
  proposalId: string,
  note?: string,
): Promise<EvalCaseProposal> {
  const response = await api.post<EvalCaseProposal>(
    `/v1/eval-proposals/${encodeURIComponent(proposalId)}/dismiss`,
    note?.trim() ? { note: note.trim() } : {},
  );
  return response.data;
}
